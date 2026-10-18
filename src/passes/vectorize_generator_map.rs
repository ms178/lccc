//! Generator-map vectorizer: pure induction-variable byte stores.
//!
//! Recognizes the counted loop with no source streams
//!
//! ```c
//! for (i = C0; i < N; ++i) dst[i] = (u8)F(i);
//! ```
//!
//! where `F` is a pure chain of 32/64-bit integer operations over `i` and
//! constants. The packed map vectorizer requires at least one load and declines
//! these loops, leaving the whole generator scalar (the histogram, byte-load and
//! ASCII-table initializers in the benchmark corpus).
//!
//! Lowering. Eight dword lanes carry eight consecutive values of `i`. Every
//! admitted operation is either a lane-wise 32-bit integer op, or a cast that
//! preserves the low 32 bits. This is exact because:
//!   * `C0 <= i < N <= 2^32`, so every `i` in the loop fits in its low 32 bits;
//!   * Add, Sub, Mul, And, Or, Xor and Shl by a constant below 32 commute with
//!     truncation mod 2^32, and the final truncation to 8 bits only reads the low
//!     8 bits of the chain;
//!   * LogicalShr is admitted only at 32-bit type, where the operand is exact.
//!
//! Exact constants. `IrConst::to_i64` truncates `I128` values outside the `i64`
//! range. Loop bounds and shift counts therefore go through [`exact_i64`], which
//! rejects such values; only lane-wise arithmetic constants, whose low 32 bits
//! are exact, use the truncating conversion.
//!
//! Shape. The vector loop is inserted between the preheader and the original
//! loop. It runs `floor((N - C0) / 32)` packed iterations and leaves the
//! induction variable at the first unprocessed element. The original scalar loop
//! then runs the tail unchanged, so no remainder is built and the scalar path is
//! the reference for the remainder.
//!
//! Lane layout. Four ymm vectors `m0..m3` cover 32 elements. Vector `j`, lane
//! `k` holds element `base + OFF[j][k]`, chosen so that
//! `packuswb(packssdw(m0,m1), packssdw(m2,m3))` already emits the 32 bytes in
//! element order. No cross-lane permute is needed.
//!
//! Every fact the transform relies on is checked by a read-only matcher before
//! the function is mutated. Lowering is pure: it builds the new blocks and the
//! preheader instructions from the unmodified function, and only a fully
//! successful lowering is committed, so an unexpected shape leaves the loop
//! scalar with no partial edit.
//!
//! Effect contract: every body instruction is accounted for. Exactly one
//! nonvolatile, default-address-space byte store and its unique GEP are
//! admitted by instruction identity; no other stores, loads, calls or asm
//! survive matching. The CFG has one body entry, and all iteration-local
//! results remain local. Rejection leaves IR and allocation cursors unchanged.
use super::*;
use crate::common::source::Span;
use crate::ir::analysis::{FlatAdj, build_cfg, build_label_map};

/// Elements per packed iteration: four ymm vectors of eight dword lanes.
const VEC_ELEMS: u64 = 32;
const GROUPS: usize = 4;

/// Element offset of lane `k` in vector `j` relative to the iteration base.
/// Chosen so that the two-level pack emits the bytes in element order.
const OFF: [[u32; 8]; GROUPS] = [
    [0, 1, 2, 3, 16, 17, 18, 19],
    [4, 5, 6, 7, 20, 21, 22, 23],
    [8, 9, 10, 11, 24, 25, 26, 27],
    [12, 13, 14, 15, 28, 29, 30, 31],
];

/// Value of an integer constant, exact for every width.
fn exact_i64(c: &IrConst) -> Option<i64> {
    match c {
        IrConst::I128(v) => i64::try_from(*v).ok(),
        other => other.to_i64(),
    }
}

/// Low 32 bits of an integer constant operand (lane-wise arithmetic only).
fn lane_const(op: &Operand) -> Option<u32> {
    match op {
        Operand::Const(c) => c.to_i64().map(|v| v as u32),
        Operand::Value(_) => None,
    }
}

/// A shift count: an exact constant below the lane width.
fn shift_count(op: &Operand) -> Option<u32> {
    match op {
        Operand::Const(c) => u32::try_from(exact_i64(c)?).ok().filter(|s| *s < 32),
        Operand::Value(_) => None,
    }
}

fn is_word_int(ty: IrType) -> bool {
    matches!(ty, IrType::U32 | IrType::I32 | IrType::U64 | IrType::I64)
}

/// Integer constant of the induction type: `I32` for U32 IVs, `I64` for U64.
fn iv_const(ty: IrType, v: u64) -> Operand {
    match ty {
        IrType::U32 => Operand::Const(IrConst::I32(v as u32 as i32)),
        _ => Operand::Const(IrConst::I64(v as i64)),
    }
}

/// One linear scan indexes definitions and use ownership. A value owned by
/// `usize::MAX` is used in multiple blocks. Escape queries are O(1), rather
/// than rescanning the entire function for every expression node. Shared CFG
/// construction includes switch, computed-goto and asm-goto edges.
struct FunctionFacts {
    defs: FxHashMap<u32, u32>,
    use_owner: FxHashMap<u32, usize>,
    labels: FxHashMap<BlockId, usize>,
    preds: FlatAdj,
}

impl FunctionFacts {
    fn build(func: &IrFunction) -> Self {
        let labels = build_label_map(func);
        let (preds, _) = build_cfg(func, &labels);
        let mut facts = Self {
            defs: FxHashMap::default(),
            use_owner: FxHashMap::default(),
            labels,
            preds,
        };
        for (bi, block) in func.blocks.iter().enumerate() {
            let mut record_use = |id| {
                facts
                    .use_owner
                    .entry(id)
                    .and_modify(|owner| {
                        if *owner != bi {
                            *owner = usize::MAX;
                        }
                    })
                    .or_insert(bi);
            };
            for inst in &block.instructions {
                if let Some(d) = inst.dest() {
                    *facts.defs.entry(d.0).or_insert(0) += 1;
                }
                if let Instruction::InlineAsm { outputs, .. } = inst {
                    for (_, dest, _) in outputs {
                        *facts.defs.entry(dest.0).or_insert(0) += 1;
                    }
                }
                inst.for_each_used_value(&mut record_use);
            }
            block.terminator.for_each_used_value(&mut record_use);
        }
        facts
    }

    fn escapes(&self, id: u32, body: usize) -> bool {
        self.use_owner.get(&id).is_some_and(|owner| *owner != body)
    }
}

/// Everything the lowering needs. Built only by a fully successful match.
struct Plan {
    body: usize,
    header: usize,
    preheader: usize,
    iv: Value,
    iv_ty: IrType,
    c0: u64,
    limit: u64,
    /// `Some(name)` when the store base is a `GlobalAddr` defined in the body;
    /// it is rematerialized in the preheader.
    base_global: Option<String>,
    base: Value,
    /// Destination of the byte-truncating cast (the stored value).
    root_cast: Value,
    /// Source of the byte-truncating cast: the vector value to narrow.
    root_src: Value,
    /// Address cast, also usable as a low-bit arithmetic alias of the IV.
    addr_cast: Value,
}

/// The counted-loop skeleton: the body ends in `iv += 1` and falls into a
/// header that tests `iv <u N`; the preheader enters the header by an
/// unconditional branch with a constant start.
struct Induction {
    header: usize,
    preheader: usize,
    iv: Value,
    iv_ty: IrType,
    c0: u64,
    limit: u64,
}

fn match_induction(func: &IrFunction, b: usize, facts: &FunctionFacts) -> Option<Induction> {
    let defs = &facts.defs;
    let blk = &func.blocks[b];
    let Terminator::Branch(hdr_label) = blk.terminator else {
        return None;
    };
    let len = blk.instructions.len();
    if len < 2 {
        return None;
    }
    let Instruction::BinOp {
        dest: inc,
        op: IrBinOp::Add,
        lhs: Operand::Value(iv),
        rhs: Operand::Const(one),
        ty: iv_ty,
    } = &blk.instructions[len - 1]
    else {
        return None;
    };
    let (inc, iv, iv_ty) = (*inc, *iv, *iv_ty);
    if one.to_i64() != Some(1) || !matches!(iv_ty, IrType::U32 | IrType::U64) {
        return None;
    }
    if defs.get(&inc.0).copied() != Some(1) || defs.get(&iv.0).copied() != Some(1) {
        return None;
    }
    let header = *facts.labels.get(&hdr_label)?;
    if header == b {
        return None;
    }
    // Header: exactly the induction phi and the exit test `iv <u N`.
    let hdr = &func.blocks[header];
    let [
        Instruction::Phi {
            dest: phi_dest,
            ty: phi_ty,
            incoming,
        },
        Instruction::Cmp {
            dest: hcmp,
            op: IrCmpOp::Ult,
            lhs: Operand::Value(h_iv),
            rhs: Operand::Const(lim_c),
            ty: h_ty,
        },
    ] = hdr.instructions.as_slice()
    else {
        return None;
    };
    if *phi_dest != iv || *phi_ty != iv_ty || *h_iv != iv || *h_ty != iv_ty {
        return None;
    }
    let (cond, h_true, h_false) = match &hdr.terminator {
        Terminator::CondBranch {
            cond: Operand::Value(c),
            true_label,
            false_label,
        } => (*c, *true_label, *false_label),
        _ => return None,
    };
    if *hcmp != cond || h_true != blk.label || h_false == hdr.label {
        return None;
    }
    // The bound must be exact and fit the 32-bit element-count contract.
    let limit = u64::try_from(exact_i64(lim_c)?)
        .ok()
        .filter(|l| *l < (1u64 << 32))?;
    // Phi: the preheader supplies a constant start; the body supplies the inc.
    let [(a0, a1), (b0, b1)] = incoming.as_slice() else {
        return None;
    };
    let (init, pred) = if *a1 == blk.label {
        if *a0 != Operand::Value(inc) {
            return None;
        }
        (b0, *b1)
    } else if *b1 == blk.label {
        if *b0 != Operand::Value(inc) {
            return None;
        }
        (a0, *a1)
    } else {
        return None;
    };
    let Operand::Const(c0) = init else {
        return None;
    };
    let c0 = u64::try_from(exact_i64(c0)?).ok()?;
    if c0 >= limit || limit - c0 < VEC_ELEMS {
        return None;
    }
    // The header's only predecessors are the preheader and this body, and the
    // preheader enters it by an unconditional branch.
    let predecessors = facts.preds.row(header);
    if predecessors.len() != 2 || facts.preds.row(b) != [header as u32] {
        return None;
    }
    let preheader = predecessors.iter().copied().find(|&bi| bi as usize != b)? as usize;
    if preheader == header || func.blocks[preheader].label != pred {
        return None;
    }
    if !matches!(func.blocks[preheader].terminator, Terminator::Branch(t) if t == hdr.label) {
        return None;
    }
    Some(Induction {
        header,
        preheader,
        iv,
        iv_ty,
        c0,
        limit,
    })
}

/// What the body match establishes beyond the induction skeleton.
struct BodyMatch {
    base: Value,
    base_global: Option<String>,
    root_cast: Value,
    root_src: Value,
    addr_cast: Value,
}

fn match_body(
    func: &IrFunction,
    b: usize,
    ind: &Induction,
    facts: &FunctionFacts,
) -> Option<BodyMatch> {
    let defs = &facts.defs;
    let blk = &func.blocks[b];
    let iv = ind.iv;
    let iv_ty = ind.iv_ty;
    // Body validation. Everything before the latch tail must be an admitted
    // vector op, the single address computation, the single byte store, or a
    // global address feeding the store base.
    let (_latch, body) = blk.instructions.split_last()?;
    let mut store: Option<(usize, Value, Value, IrType)> = None;
    let mut gep: Option<(usize, Value, Value, Value)> = None;
    for (ii, inst) in body.iter().enumerate() {
        match inst {
            Instruction::Store {
                val: Operand::Value(v),
                ptr,
                ty,
                volatile: false,
                seg_override: AddressSpace::Default,
            } => {
                if store.is_some() {
                    return None;
                }
                store = Some((ii, *v, *ptr, *ty));
            }
            Instruction::GetElementPtr {
                dest,
                base,
                offset: Operand::Value(off),
                ty: IrType::Ptr,
            } => {
                if gep.is_some() {
                    return None;
                }
                gep = Some((ii, *dest, *base, *off));
            }
            // Every memory operation must be precisely the supported one.
            // In particular a constant-valued/volatile/FS/GS store must not
            // disappear behind the computation walk's opcode whitelist.
            Instruction::Store { .. } | Instruction::GetElementPtr { .. } => return None,
            _ => {}
        }
    }
    let (store_index, store_val, store_ptr, store_ty) = store?;
    let (gep_index, gep_dest, base_v, addr_cast) = gep?;
    if gep_dest != store_ptr
        || !matches!(store_ty, IrType::U8 | IrType::I8)
        || defs.get(&gep_dest.0).copied() != Some(1)
        || defs.get(&addr_cast.0).copied() != Some(1)
    {
        return None;
    }

    let mut known: FxHashSet<u32> = FxHashSet::default();
    known.insert(iv.0);
    let mut root_cast: Option<(Value, Value)> = None; // (dest, src)
    let mut base_global: Option<String> = None;
    let mut addr_seen = false;
    for (ii, inst) in body.iter().enumerate() {
        match inst {
            _ if ii == store_index || ii == gep_index => {}
            Instruction::GlobalAddr { dest, name } => {
                // Only a global feeding the store base is admitted.
                if *dest != base_v || defs.get(&dest.0).copied() != Some(1) {
                    return None;
                }
                base_global = Some(name.clone());
            }
            Instruction::Cast {
                dest,
                src: Operand::Value(src),
                from_ty,
                to_ty,
            } => {
                if *dest == addr_cast {
                    // Address cast of the induction variable (byte offset). It
                    // is re-emitted as the same iv -> I64/U64 extension, so the
                    // source type must be the induction type exactly.
                    if addr_seen
                        || *src != iv
                        || *from_ty != iv_ty
                        || !matches!(to_ty, IrType::I64 | IrType::U64)
                    {
                        return None;
                    }
                    addr_seen = true;
                    known.insert(dest.0);
                    continue;
                }
                if *dest == store_val {
                    // The byte truncation of the stored value.
                    if root_cast.is_some()
                        || !known.contains(&src.0)
                        || !is_word_int(*from_ty)
                        || !matches!(to_ty, IrType::U8 | IrType::I8)
                        || defs.get(&dest.0).copied() != Some(1)
                    {
                        return None;
                    }
                    root_cast = Some((*dest, *src));
                    continue;
                }
                // A width-preserving cast aliases its source's lanes.
                if !known.contains(&src.0)
                    || !is_word_int(*from_ty)
                    || !is_word_int(*to_ty)
                    || defs.get(&dest.0).copied() != Some(1)
                {
                    return None;
                }
                known.insert(dest.0);
            }
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                if !is_word_int(*ty) || defs.get(&dest.0).copied() != Some(1) {
                    return None;
                }
                let lhs_vec = matches!(lhs, Operand::Value(v) if known.contains(&v.0));
                let rhs_vec = matches!(rhs, Operand::Value(v) if known.contains(&v.0));
                match op {
                    IrBinOp::Add
                    | IrBinOp::Sub
                    | IrBinOp::Mul
                    | IrBinOp::And
                    | IrBinOp::Or
                    | IrBinOp::Xor => {
                        // At least one vector operand; the other is a constant
                        // broadcast in the preheader.
                        let ok = (lhs_vec && (rhs_vec || lane_const(rhs).is_some()))
                            || (rhs_vec && lane_const(lhs).is_some());
                        if !ok {
                            return None;
                        }
                    }
                    IrBinOp::Shl | IrBinOp::LShr => {
                        // Immediate shifts below the lane width. A logical right
                        // shift needs the exact 32-bit operand.
                        if !lhs_vec
                            || (*op == IrBinOp::LShr && *ty != IrType::U32)
                            || shift_count(rhs).is_none()
                        {
                            return None;
                        }
                    }
                    _ => return None,
                }
                known.insert(dest.0);
            }
            _ => return None,
        }
    }
    let (root_cast_dest, root_src) = root_cast?;
    if !addr_seen || !known.contains(&root_src.0) {
        return None;
    }

    // Iteration-local values must not escape the body.
    for &d in &known {
        if d != iv.0 && facts.escapes(d, b) {
            return None;
        }
    }
    if facts.escapes(root_cast_dest.0, b)
        || facts.escapes(addr_cast.0, b)
        || facts.escapes(store_ptr.0, b)
    {
        return None;
    }
    // A store base defined in the body must be the admitted global address; any
    // other in-body definition would not dominate the new vector loop.
    let base_defined_in_body = body.iter().any(|i| i.dest() == Some(base_v));
    if base_defined_in_body != base_global.is_some() {
        return None;
    }

    Some(BodyMatch {
        base: base_v,
        base_global,
        root_cast: root_cast_dest,
        root_src,
        addr_cast,
    })
}

fn match_loop(func: &IrFunction, b: usize, facts: &FunctionFacts) -> Option<Plan> {
    let ind = match_induction(func, b, facts)?;
    let body = match_body(func, b, &ind, facts)?;
    Some(Plan {
        body: b,
        header: ind.header,
        preheader: ind.preheader,
        iv: ind.iv,
        iv_ty: ind.iv_ty,
        c0: ind.c0,
        limit: ind.limit,
        base_global: body.base_global,
        base: body.base,
        root_cast: body.root_cast,
        root_src: body.root_src,
        addr_cast: body.addr_cast,
    })
}

/// Sequential fresh value ids for the lowering.
struct Fresh(u32);

impl Fresh {
    fn value(&mut self) -> Value {
        let v = Value(self.0);
        self.0 += 1;
        v
    }
}

/// Preheader-hoisted operands shared by every packed iteration.
struct Hoisted {
    /// Per-vector lane offsets `OFF[j][k]` as byte-broadcast constants.
    iota: [Value; GROUPS],
    /// Broadcast of each distinct arithmetic constant in the body, by low 32 bits.
    const_vecs: FxHashMap<u32, Value>,
    /// Broadcast of 0xFF for the byte narrowing.
    mask255: Value,
}

/// The fully built replacement, ready to commit.
struct Lowered {
    viv: Value,
    pre: Vec<Instruction>,
    vh: BasicBlock,
    vb: BasicBlock,
    /// First value id not yet used by the lowering; becomes `next_value_id`.
    next_value: u32,
}

fn broadcast(dest: Value, arg: Operand) -> Instruction {
    Instruction::Intrinsic {
        dest: Some(dest),
        op: IntrinsicOp::VecBroadcastI32x8,
        dest_ptr: None,
        args: vec![arg],
    }
}

/// Preheader instructions: the store base rematerialization, lane offsets,
/// the byte mask and the arithmetic constants.
fn hoist(
    func: &IrFunction,
    plan: &Plan,
    fresh: &mut Fresh,
    pre: &mut Vec<Instruction>,
    base: &mut Value,
) -> Option<Hoisted> {
    if let Some(name) = &plan.base_global {
        let nb = fresh.value();
        pre.push(Instruction::GlobalAddr {
            dest: nb,
            name: name.clone(),
        });
        *base = nb;
    }
    let mut iota = [Value(0); GROUPS];
    for (j, slot) in iota.iter_mut().enumerate() {
        *slot = fresh.value();
        let bytes: Vec<Operand> = (0..32usize)
            .map(|b| {
                let elem = OFF[j][b / 4];
                let byte = (elem >> (8 * (b % 4))) & 0xff;
                Operand::Const(IrConst::I8(byte as u8 as i8))
            })
            .collect();
        pre.push(Instruction::Intrinsic {
            dest: Some(*slot),
            op: IntrinsicOp::VecConstI8x32,
            dest_ptr: None,
            args: bytes,
        });
    }
    let mask255 = fresh.value();
    pre.push(broadcast(mask255, Operand::Const(IrConst::I32(255))));
    let mut const_vecs: FxHashMap<u32, Value> = FxHashMap::default();
    let (_, body) = func.blocks[plan.body].instructions.split_last()?;
    for inst in body {
        let Instruction::BinOp { op, lhs, rhs, .. } = inst else {
            continue;
        };
        if matches!(op, IrBinOp::Shl | IrBinOp::LShr) {
            continue;
        }
        for operand in [lhs, rhs] {
            let Some(c) = lane_const(operand) else {
                continue;
            };
            if !const_vecs.contains_key(&c) {
                let v = fresh.value();
                pre.push(broadcast(v, Operand::Const(IrConst::I32(c as i32))));
                const_vecs.insert(c, v);
            }
        }
    }
    Some(Hoisted {
        iota,
        const_vecs,
        mask255,
    })
}

/// Operand `j`-th vector: a lane-wise value, or a hoisted constant broadcast.
fn operand_vec(
    vec_of: &FxHashMap<u32, [Value; GROUPS]>,
    hoisted: &Hoisted,
    op: &Operand,
    j: usize,
) -> Option<Operand> {
    match op {
        Operand::Value(v) => Some(Operand::Value(vec_of.get(&v.0)?[j])),
        Operand::Const(_) => Some(Operand::Value(*hoisted.const_vecs.get(&lane_const(op)?)?)),
    }
}

/// The packed loop body for one 32-element iteration.
fn lower_body(
    func: &IrFunction,
    plan: &Plan,
    hoisted: &Hoisted,
    viv: Value,
    vnext: Value,
    base: Value,
    fresh: &mut Fresh,
) -> Option<Vec<Instruction>> {
    let (_, body) = func.blocks[plan.body].instructions.split_last()?;
    let mut vb: Vec<Instruction> = Vec::new();
    // Lanes are `viv + OFF[j]` as dwords.
    let viv32 = if plan.iv_ty == IrType::U64 {
        let t = fresh.value();
        vb.push(Instruction::Cast {
            dest: t,
            src: Operand::Value(viv),
            from_ty: IrType::U64,
            to_ty: IrType::U32,
        });
        Operand::Value(t)
    } else {
        Operand::Value(viv)
    };
    let bc = fresh.value();
    vb.push(broadcast(bc, viv32));
    let mut lanes = [Value(0); GROUPS];
    for (j, slot) in lanes.iter_mut().enumerate() {
        *slot = fresh.value();
        vb.push(Instruction::Intrinsic {
            dest: Some(*slot),
            op: IntrinsicOp::VecAddI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(bc), Operand::Value(hoisted.iota[j])],
        });
    }

    // Evaluate the admitted operations per vector, keyed by IR value id.
    let mut vec_of: FxHashMap<u32, [Value; GROUPS]> = FxHashMap::default();
    vec_of.insert(plan.iv.0, lanes);
    vec_of.insert(plan.addr_cast.0, lanes);
    for inst in body {
        match inst {
            Instruction::Cast { dest, src, .. } => {
                if *dest == plan.addr_cast || *dest == plan.root_cast {
                    continue;
                }
                // Width-preserving alias of an already-computed vector.
                let Operand::Value(s) = src else {
                    return None;
                };
                let lanes_src = *vec_of.get(&s.0)?;
                vec_of.insert(dest.0, lanes_src);
            }
            Instruction::BinOp {
                dest, op, lhs, rhs, ..
            } => {
                let mut out = [Value(0); GROUPS];
                for (j, slot) in out.iter_mut().enumerate() {
                    *slot = fresh.value();
                    let a = operand_vec(&vec_of, hoisted, lhs, j)?;
                    let (iop, args) = match op {
                        IrBinOp::Shl | IrBinOp::LShr => {
                            let amount = shift_count(rhs)? as i32;
                            let iop = if *op == IrBinOp::Shl {
                                IntrinsicOp::VecShlI32x8
                            } else {
                                IntrinsicOp::VecLShrI32x8
                            };
                            (iop, vec![a, Operand::Const(IrConst::I32(amount))])
                        }
                        _ => {
                            let b = operand_vec(&vec_of, hoisted, rhs, j)?;
                            let iop = match op {
                                IrBinOp::Add => IntrinsicOp::VecAddI32x8,
                                IrBinOp::Sub => IntrinsicOp::VecSubI32x8,
                                IrBinOp::Mul => IntrinsicOp::VecMulI32x8,
                                IrBinOp::And => IntrinsicOp::VecAndI32x8,
                                IrBinOp::Or => IntrinsicOp::VecOrI32x8,
                                _ => IntrinsicOp::VecXorI32x8,
                            };
                            (iop, vec![a, b])
                        }
                    };
                    vb.push(Instruction::Intrinsic {
                        dest: Some(*slot),
                        op: iop,
                        dest_ptr: None,
                        args,
                    });
                }
                vec_of.insert(dest.0, out);
            }
            _ => {}
        }
    }

    // Narrow each dword to a byte, then the two-level pack. The lane layout
    // makes the packed result already in element order.
    let src = *vec_of.get(&plan.root_src.0)?;
    let mut masked = [Value(0); GROUPS];
    for (j, slot) in masked.iter_mut().enumerate() {
        *slot = fresh.value();
        vb.push(Instruction::Intrinsic {
            dest: Some(*slot),
            op: IntrinsicOp::VecAndI32x8,
            dest_ptr: None,
            args: vec![Operand::Value(src[j]), Operand::Value(hoisted.mask255)],
        });
    }
    let t01 = fresh.value();
    let t23 = fresh.value();
    let off = fresh.value();
    let gep = fresh.value();
    // Pure SSA narrowing exposes register allocation and load folding to
    // the backend: no scratch allocas, stores or reloads are required by IR.
    for (dest, pair) in [(t01, 0usize), (t23, 2)] {
        vb.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::VecPackssdwI16x16,
            dest_ptr: None,
            args: vec![
                Operand::Value(masked[pair]),
                Operand::Value(masked[pair + 1]),
            ],
        });
    }

    // Level two: words to bytes, written straight to base + viv (32 bytes).
    vb.push(Instruction::Cast {
        dest: off,
        src: Operand::Value(viv),
        from_ty: plan.iv_ty,
        to_ty: IrType::I64,
    });
    vb.push(Instruction::GetElementPtr {
        dest: gep,
        base,
        offset: Operand::Value(off),
        ty: IrType::Ptr,
    });
    vb.push(Instruction::Intrinsic {
        dest: None,
        op: IntrinsicOp::Packuswb256,
        dest_ptr: Some(gep),
        args: vec![Operand::Value(t01), Operand::Value(t23)],
    });

    // Advance the packed induction variable (fed back through the vector phi).
    vb.push(Instruction::BinOp {
        dest: vnext,
        op: IrBinOp::Add,
        lhs: Operand::Value(viv),
        rhs: iv_const(plan.iv_ty, VEC_ELEMS),
        ty: plan.iv_ty,
    });
    Some(vb)
}

/// Build the replacement blocks without touching `func`.
fn lower(
    func: &IrFunction,
    plan: &Plan,
    next_value: u32,
    vh_label: BlockId,
    vb_label: BlockId,
) -> Option<Lowered> {
    let mut fresh = Fresh(next_value);
    let pre_label = func.blocks[plan.preheader].label;
    let header_label = func.blocks[plan.header].label;
    let kend = plan.c0 + ((plan.limit - plan.c0) / VEC_ELEMS) * VEC_ELEMS;

    let viv = fresh.value();
    let vc = fresh.value();
    let vnext = fresh.value();

    let mut pre: Vec<Instruction> = Vec::new();
    let mut base = plan.base;
    let hoisted = hoist(func, plan, &mut fresh, &mut pre, &mut base)?;
    let vb_insts = lower_body(func, plan, &hoisted, viv, vnext, base, &mut fresh)?;

    // Vector header: the packed induction phi and the packed bound test.
    let vh = BasicBlock {
        label: vh_label,
        instructions: vec![
            Instruction::Phi {
                dest: viv,
                ty: plan.iv_ty,
                incoming: vec![
                    (iv_const(plan.iv_ty, plan.c0), pre_label),
                    (Operand::Value(vnext), vb_label),
                ],
            },
            Instruction::Cmp {
                dest: vc,
                op: IrCmpOp::Ult,
                lhs: Operand::Value(viv),
                rhs: iv_const(plan.iv_ty, kend),
                ty: plan.iv_ty,
            },
        ],
        terminator: Terminator::CondBranch {
            cond: Operand::Value(vc),
            true_label: vb_label,
            false_label: header_label,
        },
        source_spans: vec![],
    };
    let vb = BasicBlock {
        label: vb_label,
        instructions: vb_insts,
        terminator: Terminator::Branch(vh_label),
        source_spans: vec![],
    };
    Some(Lowered {
        viv,
        pre,
        vh,
        vb,
        next_value: fresh.0,
    })
}

/// Commit a successful lowering: extend the preheader, route the scalar header
/// phi through the vector loop, and append the two new blocks.
fn commit(func: &mut IrFunction, plan: &Plan, low: Lowered) {
    let pre_label = func.blocks[plan.preheader].label;
    let vh_label = low.vh.label;
    let next_value = low.next_value;
    {
        let p = &mut func.blocks[plan.preheader];
        let had_spans = !p.source_spans.is_empty();
        let added = low.pre.len();
        p.instructions.extend(low.pre);
        if had_spans {
            p.source_spans.extend((0..added).map(|_| Span {
                start: 0,
                end: 0,
                file_id: 0,
            }));
        }
        p.terminator = Terminator::Branch(vh_label);
    }
    // The scalar loop's header phi now receives the packed exit value from the
    // vector header, instead of the preheader.
    for inst in &mut func.blocks[plan.header].instructions {
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if *dest == plan.iv {
                for (val, from) in incoming.iter_mut() {
                    if *from == pre_label {
                        *val = Operand::Value(low.viv);
                        *from = vh_label;
                    }
                }
            }
        }
    }
    func.next_label = low.vb.label.0 + 1;
    func.blocks.push(low.vh);
    func.blocks.push(low.vb);
    func.next_value_id = next_value;
}

/// Run the generator-map transform on the first matching loop of `func`.
///
/// `trace` enables the `[VEC-GEN] fn=` line that `check_generator_map.sh`
/// counts. Returns the number of transformed loops (0 or 1).
pub(super) fn transform_function(func: &mut IrFunction, trace: bool) -> usize {
    let facts = FunctionFacts::build(func);
    if facts.labels.len() != func.blocks.len() {
        return 0;
    }
    let Some(plan) = (0..func.blocks.len()).find_map(|b| match_loop(func, b, &facts)) else {
        return 0;
    };
    let next = func.sound_next_value_id();
    // Bound all lowering allocations before constructing temporary blocks.
    // Eight fresh IDs per scalar instruction plus 128 fixed IDs is a strict
    // overestimate (at most two broadcasts + four lane results per binop).
    let Some(budget) = u32::try_from(func.blocks[plan.body].instructions.len())
        .ok()
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(128))
    else {
        return 0;
    };
    if next.checked_add(budget).is_none() {
        return 0;
    }
    let max_label = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
    let Some(first_label) = max_label.checked_add(1).map(|n| n.max(func.next_label)) else {
        return 0;
    };
    if first_label.checked_add(2).is_none() {
        return 0;
    }
    let (vh_label, vb_label) = (BlockId(first_label), BlockId(first_label + 1));
    let Some(low) = lower(func, &plan, next, vh_label, vb_label) else {
        return 0;
    };
    if trace {
        eprintln!(
            "[VEC-GEN] fn={} body=B{} header=H{} N={} C0={} iv_ty={:?}: packed generator loop",
            func.name, plan.body, plan.header, plan.limit, plan.c0, plan.iv_ty
        );
    }
    commit(func, &plan, low);
    1
}

#[cfg(test)]
#[path = "vectorize_generator_map_tests.rs"]
mod tests;
