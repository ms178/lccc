//! Sparse small-set membership: fold chains of `x == C` / range tests into
//! one bit-mask test — GCC's `subl $45,%edi; cmpb $50,%dil; ja miss;
//! btq %rdi,MASK` classify idiom (Expat xml_name_continue, SQLite varint).
//!
//! Shape recognized (post range_fold + cfg_simplify — hit blocks are already
//! merged into the join Phi):
//!
//! ```text
//! T_i:  <pure test insts>                 ; Eq / range / other pure test
//!       CondBranch cond -> JOIN, T_{i+1}  ; hit edge straight to the phi
//! ...
//! TAIL: Cmp x, C_n                        ; optional value-block tail
//!       Branch JOIN                       ; phi uses the Cmp value
//! JOIN: Phi [(Const(1), T_1), ..., (Value(tail_cmp), TAIL), ...]
//! ```
//!
//! Member kinds:
//!  * `Eq`: exactly `Cmp Eq x, C; CondBranch(cmp)` — one mask bit.
//!  * `Range` (range_fold output): `[Cast x→I32]; Sub c, lo; Cmp Ule sub,
//!    span; [Cast]; CondBranch` — a run of mask bits.
//!  * `Skip`: any other single `Cmp <op> x, C; CondBranch(cmp)` — not
//!    maskable, but every test in the chain is PURE and the chain computes
//!    a disjunction, so `||` commutativity lets mask members be collected
//!    ACROSS skips (`c >= 0xc2U` sits mid-chain in Expat's classifier and
//!    used to split one foldable set into two half-size clusters).
//!  * Tail value block: `Cmp <op> x, C; Branch(JOIN)` with the Phi
//!    consuming the Cmp value — `Eq` tails are maskable, others act as
//!    skip terminators.
//!
//! Transformations (one per call; the driver's fixpoint loop iterates):
//!
//! 1. CASE-FOLD PAIR MERGE: two `Range` members `[a,b]` and `[a+32,b+32]`
//!    with `(a & 32) == 0` and `a >> 5 == b >> 5` (so bit 5 is clear across
//!    the whole low range) merge into ONE test on `x & ~32`:
//!    `(u32)((x & 0xffffffdf) - a) <= b - a` — GCC's `andl $-33` ASCII
//!    letter fold. Exact for every 32-bit input: clearing bit 5 maps
//!    `[a+32,b+32]` onto `[a,b]` and maps nothing else into it.
//!
//! 2. BIT-MASK CLUSTER: choose the window of width ≤ 63 (31 on 32-bit
//!    targets) containing the MOST maskable members (Eq bits and whole
//!    Ranges). With ≥ 3 members, rewrite:
//!
//!    ```text
//!    B1(head):  sub = (i32)x - lo ; CondBranch (u32)sub > span -> MISSES, B2
//!    B2(new):   bit = BitTest MASK, sub
//!               mode A: CondBranch bit -> JOIN, MISSES
//!               mode B: Branch JOIN            ; phi += (bit, B2)
//!    ```
//!
//!    where MISSES chains through the surviving skip tests. When the TAIL
//!    was absorbed into the mask, the last surviving test converts into the
//!    new value-carrying tail (its 0/1 condition value feeds the Phi
//!    directly); with no survivors at all, B2 itself is the value carrier
//!    (mode B) and B1's guard miss feeds the Phi with Const(0).
//!
//! Soundness:
//!  * every member block contains ONLY its test pattern; every def is used
//!    only inside its own block (tail Cmp: plus exactly the one Phi use);
//!  * every member except the first has exactly one predecessor (the
//!    previous member's miss edge), so no other path enters mid-chain;
//!  * all tests read the same root value; reordering is sound because each
//!    is pure and the chain is a disjunction;
//!  * BitTest is emitted under a range guard, and the backend's BT masks
//!    its count at machine level — no out-of-range UB is reachable;
//!  * absorbed blocks lose their only entry edge and die in cfg_simplify's
//!    unreachable sweep; their Phi entries are removed here.
//!
//! Pass name for CCC_DISABLE_PASSES: "set_membership".

use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};

/// Classification of one chain member.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `x == C` — one mask bit at C.
    Eq(i64),
    /// `(u32)(x - lo) <= hi - lo` — mask bits lo..=hi.
    Range(i64, i64),
    /// Any other pure single-compare test — kept, never masked.
    Skip,
}

struct Member {
    /// Block index in `func.blocks`.
    bi: usize,
    /// Block label (phi incoming key / edge target).
    label: BlockId,
    kind: Kind,
    /// False-edge target (None for the tail value block).
    miss: Option<BlockId>,
    /// Tail value block: the Phi consumes this block's condition value.
    is_tail: bool,
    /// The branch condition / tail value id (for tail-conversion rewiring).
    cond_val: u32,
    /// Number of leading instructions that are NOT part of the test and
    /// must be preserved by any rewrite (function-entry prelude: Alloca /
    /// ParamRef / the shared zext). Non-zero only for the chain head.
    prelude_len: usize,
}

/// Sign-normalize an equality constant into the unsigned domain of the
/// compare type: U8 compares store `IrConst::I8(-62)` for 0xc2, and the
/// membership arithmetic runs on the zero-extended value.
fn norm_const(c: &IrConst, ty: IrType) -> Option<i64> {
    let raw = c.to_i64()?;
    Some(match ty {
        IrType::U8 => raw as u8 as i64,
        IrType::U16 => raw as u16 as i64,
        IrType::U32 => raw as u32 as i64,
        IrType::I32 => raw as i32 as i64,
        _ => return None,
    })
}

/// Root types the fold understands: unsigned narrow (zero-extension domain)
/// or 32-bit. Signed narrow roots are excluded — their widening cast
/// sign-extends and the rebase arithmetic would disagree with `norm_const`.
fn root_ty_ok(ty: IrType) -> bool {
    matches!(ty, IrType::U8 | IrType::U16 | IrType::I32 | IrType::U32)
}

/// Shape check: does `insts` form a complete test (single Cmp, or the
/// [Cast]; Sub; Cmp Ule; [Cast] range shape) whose defs are all internal
/// except the final condition? Used to find the head block's test suffix.
fn suffix_parses(insts: &[Instruction]) -> bool {
    match insts.len() {
        1 => matches!(insts[0], Instruction::Cmp { .. }),
        2..=4 => {
            let mut idx = 0;
            if matches!(insts[idx], Instruction::Cast { .. }) {
                idx += 1;
            }
            if !matches!(
                insts.get(idx),
                Some(Instruction::BinOp { op: IrBinOp::Sub, .. })
            ) {
                return false;
            }
            idx += 1;
            if !matches!(insts.get(idx), Some(Instruction::Cmp { op: IrCmpOp::Ule, .. })) {
                return false;
            }
            idx += 1;
            if matches!(insts.get(idx), Some(Instruction::Cast { .. })) {
                idx += 1;
            }
            idx == insts.len()
        }
        _ => false,
    }
}

/// Parse one member block. `expect_root` pins the compared value after the
/// first member; `join` pins the hit target.
fn parse_member(
    func: &IrFunction,
    bi: usize,
    expect_root: Option<(u32, IrType)>,
    join: Option<BlockId>,
) -> Option<(u32, IrType, Member)> {
    let block = &func.blocks[bi];
    let insts = &block.instructions;
    if insts.is_empty() {
        return None;
    }

    // Tail value block: `Cmp <op> x, C ; Branch(join)`.
    if insts.len() == 1 {
        if let (
            Instruction::Cmp { dest, op, lhs, rhs, ty },
            Terminator::Branch(t),
        ) = (&insts[0], &block.terminator)
        {
            if join.is_some_and(|j| j != *t) || !root_ty_ok(*ty) {
                return None;
            }
            let (val, con) = match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(c)) => (v.0, norm_const(c, *ty)),
                (Operand::Const(c), Operand::Value(v)) => (v.0, norm_const(c, *ty)),
                _ => return None,
            };
            if expect_root.is_some_and(|(r, rt)| r != val || rt != *ty) {
                return None;
            }
            let kind = match (op, con) {
                (IrCmpOp::Eq, Some(c)) => Kind::Eq(c),
                _ => Kind::Skip,
            };
            return Some((
                val,
                *ty,
                Member {
                    bi,
                    label: block.label,
                    kind,
                    miss: None,
                    is_tail: true,
                    cond_val: dest.0,
                    prelude_len: 0,
                },
            ));
        }
    }

    let Terminator::CondBranch { cond: Operand::Value(cv), true_label, false_label } =
        &block.terminator
    else {
        return None;
    };
    if join.is_some_and(|j| j != *true_label) {
        return None;
    }

    // Head blocks may carry a PRELUDE (function entry: Alloca, ParamRef,
    // the root's own zext, ...) before the test. The test is parsed as a
    // SUFFIX; prelude instructions stay in place across the rewrite, so
    // their defs may be used anywhere (they are not deleted). Only the
    // chain HEAD gets this liberty: interior members must remain fully
    // absorbable.
    let is_head = expect_root.is_none();
    let suffix_start = if is_head {
        // Longest parseable suffix: try the last 1..=4 instructions.
        let mut start = insts.len();
        for take in 1..=4usize.min(insts.len()) {
            let cand = insts.len() - take;
            if suffix_parses(&insts[cand..]) {
                start = cand;
            }
        }
        start
    } else {
        0
    };
    let prelude_len = if is_head { suffix_start } else { 0 };
    let insts = &insts[suffix_start..];
    if insts.is_empty() {
        return None;
    }

    // Single-compare test: Eq -> maskable, anything else -> Skip.
    if insts.len() == 1 {
        let Instruction::Cmp { dest, op, lhs, rhs, ty } = &insts[0] else {
            return None;
        };
        if dest.0 != cv.0 || !root_ty_ok(*ty) {
            return None;
        }
        let (val, con) = match (lhs, rhs) {
            (Operand::Value(v), Operand::Const(c)) => (v.0, norm_const(c, *ty)),
            (Operand::Const(c), Operand::Value(v)) => (v.0, norm_const(c, *ty)),
            _ => return None,
        };
        if expect_root.is_some_and(|(r, rt)| r != val || rt != *ty) {
            return None;
        }
        let kind = match (op, con) {
            (IrCmpOp::Eq, Some(c)) => Kind::Eq(c),
            _ => Kind::Skip,
        };
        return Some((
            val,
            *ty,
            Member {
                bi,
                label: block.label,
                kind,
                miss: Some(*false_label),
                is_tail: false,
                cond_val: dest.0,
                prelude_len,
            },
        ));
    }

    // Range test (range_fold output), 2-4 instructions:
    //   [Cast c = zext x] ; Sub s = c - LO ; Cmp b = ULe s, SPAN ;
    //   [Cast w = widen b] — branch cond is w (or b without the cast).
    if insts.len() > 4 {
        return None;
    }
    let mut idx = 0usize;
    let mut cast_dest: Option<u32> = None;
    let mut root_val: Option<(u32, IrType)> = None;
    if let Instruction::Cast { dest, src: Operand::Value(s), from_ty, to_ty } = &insts[idx] {
        // Only ZERO-extending widenings keep the unsigned domain aligned
        // with norm_const; a signed narrow source would sign-extend.
        if matches!(from_ty, IrType::U8 | IrType::U16)
            && to_ty.is_integer()
            && to_ty.size() >= from_ty.size()
        {
            cast_dest = Some(dest.0);
            root_val = Some((s.0, *from_ty));
            idx += 1;
        }
    }
    let Some(Instruction::BinOp { dest: sub_d, op: IrBinOp::Sub, lhs, rhs, .. }) = insts.get(idx)
    else {
        return None;
    };
    idx += 1;
    let lo = match rhs {
        Operand::Const(c) => c.to_i64()?,
        _ => return None,
    };
    match lhs {
        Operand::Value(v) => match cast_dest {
            Some(cd) if cd == v.0 => {}
            None => {
                root_val = Some((v.0, IrType::I32));
            }
            _ => return None,
        },
        _ => return None,
    }
    let Some(Instruction::Cmp { dest: cmp_d, op: IrCmpOp::Ule, lhs: cl, rhs: cr, .. }) =
        insts.get(idx)
    else {
        return None;
    };
    idx += 1;
    if !matches!(cl, Operand::Value(v) if v.0 == sub_d.0) {
        return None;
    }
    let span = match cr {
        Operand::Const(c) => c.to_i64()?,
        _ => return None,
    };
    if span < 0 {
        return None;
    }
    let mut cond_src = cmp_d.0;
    if let Some(Instruction::Cast { dest, src: Operand::Value(s), .. }) = insts.get(idx) {
        if s.0 == cmp_d.0 {
            cond_src = dest.0;
            idx += 1;
        }
    }
    if idx != insts.len() || cv.0 != cond_src {
        return None;
    }
    let (rv, rt) = root_val?;
    if !root_ty_ok(rt) {
        return None;
    }
    // Root identity across member kinds: eq tests compare the NARROW value,
    // range tests go through their own zext of it. Match the value id; the
    // domain agreement is enforced by root_ty_ok + zext-only casts.
    if let Some((r, _)) = expect_root {
        if r != rv {
            return None;
        }
    }
    Some((
        rv,
        rt,
        Member {
            bi,
            label: block.label,
            kind: Kind::Range(lo, lo.checked_add(span)?),
            miss: Some(*false_label),
            is_tail: false,
            cond_val: cond_src,
            prelude_len,
        },
    ))
}

pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 3 {
        return 0;
    }

    // Predecessor counts and label → index map.
    let mut pred_count: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        let mut bump = |b: BlockId| *pred_count.entry(b.0).or_insert(0) += 1;
        match &block.terminator {
            Terminator::Branch(t) => bump(*t),
            Terminator::CondBranch { true_label, false_label, .. } => {
                bump(*true_label);
                bump(*false_label);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, t) in cases {
                    bump(*t);
                }
                bump(*default);
            }
            Terminator::IndirectBranch { possible_targets, .. } => {
                for t in possible_targets {
                    bump(*t);
                }
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
    }
    let label_to_idx: FxHashMap<u32, usize> =
        func.blocks.iter().enumerate().map(|(i, b)| (b.label.0, i)).collect();

    // value id → blocks that use it (terminator + phi uses included).
    let mut use_sites: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| use_sites.entry(v).or_default().push(bi));
        }
        let mut term_use = |op: &Operand| {
            if let Operand::Value(v) = op {
                use_sites.entry(v.0).or_default().push(bi);
            }
        };
        match &block.terminator {
            Terminator::CondBranch { cond, .. } => term_use(cond),
            Terminator::Switch { val, .. } => term_use(val),
            Terminator::Return(Some(op)) => term_use(op),
            Terminator::IndirectBranch { target, .. } => term_use(target),
            _ => {}
        }
    }

    // Every def of a member block must be used only inside that block; the
    // tail's condition may additionally be used exactly by the join phi.
    let defs_local = |func: &IrFunction,
                      bi: usize,
                      skip_prefix: usize,
                      allow: Option<(u32, usize)>|
     -> bool {
        for inst in func.blocks[bi].instructions.iter().skip(skip_prefix) {
            if let Some(d) = inst.dest() {
                if let Some(uses) = use_sites.get(&d.0) {
                    for &ub in uses {
                        if ub == bi {
                            continue;
                        }
                        match allow {
                            Some((v, j)) if v == d.0 && ub == j => continue,
                            _ => return false,
                        }
                    }
                }
            }
        }
        true
    };

    let max_span: i64 = if crate::common::types::target_is_32bit() { 31 } else { 63 };

    let mut changes = 0usize;
    let mut bi = 0usize;
    while bi < func.blocks.len() {
        let Some((root, root_ty, first)) = parse_member(func, bi, None, None) else {
            bi += 1;
            continue;
        };
        if first.is_tail {
            bi += 1;
            continue;
        }
        let join = match &func.blocks[first.bi].terminator {
            Terminator::CondBranch { true_label, .. } => *true_label,
            _ => unreachable!("parse_member guarantees CondBranch"),
        };
        let Some(&join_idx) = label_to_idx.get(&join.0) else {
            bi += 1;
            continue;
        };
        // Exactly one Phi in the join so the edge retargeting below stays
        // single-writer.
        let phis: Vec<usize> = func.blocks[join_idx]
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(i, inst)| matches!(inst, Instruction::Phi { .. }).then_some(i))
            .collect();
        if phis.len() != 1 {
            bi += 1;
            continue;
        }
        let phi_pos = phis[0];
        let (phi_ty, phi_incoming) = match &func.blocks[join_idx].instructions[phi_pos] {
            Instruction::Phi { ty, incoming, .. } => (*ty, incoming.clone()),
            _ => unreachable!(),
        };
        if !phi_ty.is_integer() {
            bi += 1;
            continue;
        }
        let const1_from = |label: BlockId| {
            phi_incoming
                .iter()
                .any(|(op, b)| *b == label && matches!(op, Operand::Const(c) if c.to_i64() == Some(1)))
        };
        if !const1_from(first.label) || !defs_local(func, first.bi, first.prelude_len, None) {
            bi += 1;
            continue;
        }

        // ── Walk the chain, continuing over Skip members ──
        let mut chain: Vec<Member> = vec![first];
        loop {
            let last = chain.last().unwrap();
            let Some(miss) = last.miss else { break };
            let Some(&next_idx) = label_to_idx.get(&miss.0) else { break };
            if pred_count.get(&miss.0).copied().unwrap_or(0) != 1 {
                break;
            }
            let Some((_, _, m)) = parse_member(func, next_idx, Some((root, root_ty)), Some(join))
            else {
                break;
            };
            if m.is_tail {
                let phi_takes_value = phi_incoming.iter().any(|(op, b)| {
                    *b == m.label && matches!(op, Operand::Value(v) if v.0 == m.cond_val)
                });
                if phi_takes_value && defs_local(func, next_idx, 0, Some((m.cond_val, join_idx))) {
                    chain.push(m);
                }
                break;
            }
            if !const1_from(m.label) || !defs_local(func, next_idx, 0, None) {
                break;
            }
            chain.push(m);
        }

        // ── Transform 1: case-fold pair merge ──
        // Two ranges [a,b] and [a+32,b+32] with bit 5 clear across [a,b]
        // merge into one test on x & ~32. Absorb the LATER of the pair.
        {
            let mut fold: Option<(usize, usize, i64, i64)> = None; // (early, late, a, b)
            'search: for i in 0..chain.len() {
                let Kind::Range(a1, b1) = chain[i].kind else { continue };
                if chain[i].is_tail {
                    continue;
                }
                for j in i + 1..chain.len() {
                    let Kind::Range(a2, b2) = chain[j].kind else { continue };
                    if chain[j].is_tail {
                        continue;
                    }
                    let (lo_a, lo_b, hi_a, hi_b) =
                        if a1 < a2 { (a1, b1, a2, b2) } else { (a2, b2, a1, b1) };
                    if hi_a == lo_a + 32
                        && hi_b == lo_b + 32
                        && (lo_a & 32) == 0
                        && lo_a >= 0
                        && (lo_a >> 5) == (lo_b >> 5)
                    {
                        fold = Some((i, j, lo_a, lo_b));
                        break 'search;
                    }
                }
            }
            if let Some((i, j, a, b)) = fold {
                let mut next_id = func.next_value_id;
                if next_id == 0 {
                    next_id = func.max_value_id() + 1;
                }
                let mut fresh = || {
                    let v = Value(next_id);
                    next_id += 1;
                    v
                };
                // Rewrite the EARLIER block to the folded test; the later
                // block is absorbed (its predecessor's miss edge skips it).
                let (early, late) = (i.min(j), i.max(j));
                let early_bi = chain[early].bi;
                let mut insts: Vec<Instruction> = Vec::with_capacity(5);
                let root32 = if root_ty.size() < 4 {
                    let c = fresh();
                    insts.push(Instruction::Cast {
                        dest: c,
                        src: Operand::Value(Value(root)),
                        from_ty: root_ty,
                        to_ty: IrType::I32,
                    });
                    c
                } else {
                    Value(root)
                };
                let and_v = fresh();
                insts.push(Instruction::BinOp {
                    dest: and_v,
                    op: IrBinOp::And,
                    lhs: Operand::Value(root32),
                    rhs: Operand::Const(IrConst::I32(!32)),
                    ty: IrType::I32,
                });
                let sub_v = fresh();
                insts.push(Instruction::BinOp {
                    dest: sub_v,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(and_v),
                    rhs: Operand::Const(IrConst::I32(a as i32)),
                    ty: IrType::I32,
                });
                let cmp_v = fresh();
                insts.push(Instruction::Cmp {
                    dest: cmp_v,
                    op: IrCmpOp::Ule,
                    lhs: Operand::Value(sub_v),
                    rhs: Operand::Const(IrConst::I32((b - a) as i32)),
                    ty: IrType::U32,
                });
                // The absorbed member's successor becomes the early block's
                // miss target; when the LATE member is the early one's direct
                // successor this simply skips it.
                let late_succ = chain[late].miss.expect("non-tail member has a miss edge");
                let early_miss = if late == early + 1 {
                    late_succ
                } else {
                    chain[early + 1].label
                };
                let keep = chain[early].prelude_len;
                let mut new_insts: Vec<Instruction> =
                    func.blocks[early_bi].instructions[..keep].to_vec();
                new_insts.extend(insts);
                func.blocks[early_bi].instructions = new_insts;
                func.blocks[early_bi].source_spans.clear();
                func.blocks[early_bi].terminator = Terminator::CondBranch {
                    cond: Operand::Value(cmp_v),
                    true_label: join,
                    false_label: early_miss,
                };
                // Bypass the late member: its predecessor's miss edge jumps
                // to the late member's successor.
                if late != early + 1 {
                    let pred_bi = chain[late - 1].bi;
                    if let Terminator::CondBranch { false_label, .. } =
                        &mut func.blocks[pred_bi].terminator
                    {
                        *false_label = late_succ;
                    }
                }
                // Remove the absorbed member's phi entry.
                if let Instruction::Phi { incoming, .. } =
                    &mut func.blocks[join_idx].instructions[phi_pos]
                {
                    let late_label = chain[late].label;
                    incoming.retain(|(_, b)| *b != late_label);
                }
                func.next_value_id = next_id;
                changes += 1;
                // One transformation per call: edges were rewired, so the
                // predecessor counts computed at entry are stale. The
                // driver's fixpoint loop re-enters for the cluster step.
                return changes;
            }
        }

        // ── Transform 2: best-window bit-mask cluster ──
        // Collect maskable members and pick the window of width <= max_span
        // holding the most of them.
        let maskable: Vec<usize> = chain
            .iter()
            .enumerate()
            .filter(|(_, m)| matches!(m.kind, Kind::Eq(_) | Kind::Range(_, _)))
            .map(|(i, _)| i)
            .collect();
        if maskable.len() < 3 {
            bi = chain.last().unwrap().bi + 1;
            continue;
        }
        let bounds = |k: Kind| -> (i64, i64) {
            match k {
                Kind::Eq(c) => (c, c),
                Kind::Range(l, h) => (l, h),
                Kind::Skip => unreachable!(),
            }
        };
        // Candidate window starts: each member's lo.
        let mut best: Vec<usize> = Vec::new();
        for &wi in &maskable {
            let (w_lo, _) = bounds(chain[wi].kind);
            let inside: Vec<usize> = maskable
                .iter()
                .copied()
                .filter(|&mi| {
                    let (l, h) = bounds(chain[mi].kind);
                    l >= w_lo && h - w_lo <= max_span
                })
                .collect();
            if inside.len() > best.len() {
                best = inside;
            }
        }
        if best.len() < 3 {
            bi = chain.last().unwrap().bi + 1;
            continue;
        }
        let absorbed = best;
        let lo = absorbed.iter().map(|&i| bounds(chain[i].kind).0).min().unwrap();
        let hi = absorbed.iter().map(|&i| bounds(chain[i].kind).1).max().unwrap();
        let span = hi - lo;
        let mut mask: u64 = 0;
        for &i in &absorbed {
            let (l, h) = bounds(chain[i].kind);
            for c in l..=h {
                mask |= 1u64 << (c - lo);
            }
        }
        let use_32 = span <= 31;
        let head = absorbed[0];
        let tail_absorbed = chain.last().map(|m| m.is_tail).unwrap_or(false)
            && absorbed.contains(&(chain.len() - 1));

        // Survivors after the cluster head, in chain order.
        let live: Vec<usize> = (0..chain.len())
            .filter(|i| !absorbed.contains(i) && *i != head)
            .collect();
        // Live members BEFORE the head keep their position; the cluster
        // replaces the head block in place, so their edges are already
        // correct unless their successor got absorbed (rewired below).

        let mut next_id = func.next_value_id;
        if next_id == 0 {
            next_id = func.max_value_id() + 1;
        }
        let mut fresh = || {
            let v = Value(next_id);
            next_id += 1;
            v
        };
        let mut next_label = func.next_label;
        {
            let max_present = func.blocks.iter().map(|b| b.label.0).max().unwrap_or(0);
            if next_label <= max_present {
                next_label = max_present + 1;
            }
        }
        let b2_label = BlockId(next_label);
        next_label += 1;

        // next-live-target(i): first surviving member after chain position i;
        // the cluster head counts as a target for positions before it.
        let next_target_after = |pos: usize| -> Option<BlockId> {
            for i in pos + 1..chain.len() {
                if i == head {
                    return Some(chain[head].label);
                }
                if live.contains(&i) {
                    return Some(chain[i].label);
                }
            }
            None
        };
        // Chain END for miss edges once every later member is absorbed:
        //  * tail absorbed → the Phi handles it (value-carrier conversion);
        //  * otherwise → the original chain exit (last member's miss).
        let chain_exit: Option<BlockId> = chain.last().and_then(|m| m.miss);

        // The last survivor converts to the value-carrying tail when the
        // original tail was absorbed.
        let last_live_pos: Option<usize> = live.iter().copied().max();

        // ── Rewire surviving members' miss edges ──
        for &li in &live {
            if chain[li].is_tail {
                continue; // tail has no miss edge
            }
            let target = next_target_after(li);
            let is_last_live_after_head = tail_absorbed
                && Some(li) == last_live_pos
                && li > head;
            let m_bi = chain[li].bi;
            if target.is_none() && is_last_live_after_head {
                // Convert to value-carrying tail: Branch(join), phi takes
                // the 0/1 condition value.
                let cond_val = chain[li].cond_val;
                func.blocks[m_bi].terminator = Terminator::Branch(join);
                if let Instruction::Phi { incoming, .. } =
                    &mut func.blocks[join_idx].instructions[phi_pos]
                {
                    let lbl = chain[li].label;
                    incoming.retain(|(_, b)| *b != lbl);
                    incoming.push((Operand::Value(Value(cond_val)), lbl));
                }
            } else {
                let new_miss = target
                    .or(chain_exit)
                    .expect("non-tail chain has an exit or a later target");
                if let Terminator::CondBranch { false_label, .. } =
                    &mut func.blocks[m_bi].terminator
                {
                    *false_label = new_miss;
                }
            }
        }

        // Cluster miss target: first survivor after the head, else END.
        let cluster_next = next_target_after(head);
        let mode_b = cluster_next.is_none() && tail_absorbed;

        // ── Build B1 (head block) and B2 (appended) ──
        let mut b1: Vec<Instruction> = Vec::with_capacity(4);
        let root32 = if root_ty.size() < 4 {
            let c = fresh();
            b1.push(Instruction::Cast {
                dest: c,
                src: Operand::Value(Value(root)),
                from_ty: root_ty,
                to_ty: IrType::I32,
            });
            c
        } else {
            Value(root)
        };
        let sub_v = fresh();
        b1.push(Instruction::BinOp {
            dest: sub_v,
            op: IrBinOp::Sub,
            lhs: Operand::Value(root32),
            rhs: Operand::Const(IrConst::I32(lo as i32)),
            ty: IrType::I32,
        });
        let out_v = fresh();
        b1.push(Instruction::Cmp {
            dest: out_v,
            op: IrCmpOp::Ugt,
            lhs: Operand::Value(sub_v),
            rhs: Operand::Const(IrConst::I32(span as i32)),
            ty: IrType::U32,
        });

        let mut b2: Vec<Instruction> = Vec::with_capacity(3);
        let (bt_ty, mask_const, idx_v) = if use_32 {
            (IrType::I32, IrConst::I32(mask as u32 as i32), sub_v)
        } else {
            let z = fresh();
            b2.push(Instruction::Cast {
                dest: z,
                src: Operand::Value(sub_v),
                from_ty: IrType::U32,
                to_ty: IrType::I64,
            });
            (IrType::I64, IrConst::I64(mask as i64), z)
        };
        let bit_v = fresh();
        b2.push(Instruction::BinOp {
            dest: bit_v,
            op: IrBinOp::BitTest,
            lhs: Operand::Const(mask_const),
            rhs: Operand::Value(idx_v),
            ty: bt_ty,
        });

        let head_bi = chain[head].bi;
        let head_label = chain[head].label;

        // Remove ALL absorbed members' phi entries (head included — its hit
        // now comes from B2).
        if let Instruction::Phi { incoming, .. } =
            &mut func.blocks[join_idx].instructions[phi_pos]
        {
            incoming.retain(|(_, b)| !absorbed.iter().any(|&i| chain[i].label == *b));
        }

        let (guard_miss, b2_term) = if mode_b {
            // Guard miss feeds the phi with 0; B2 feeds it with the bit.
            let result = if bt_ty == phi_ty {
                bit_v
            } else {
                let r = fresh();
                b2.push(Instruction::Cast {
                    dest: r,
                    src: Operand::Value(bit_v),
                    from_ty: bt_ty,
                    to_ty: phi_ty,
                });
                r
            };
            if let Instruction::Phi { incoming, .. } =
                &mut func.blocks[join_idx].instructions[phi_pos]
            {
                incoming.push((Operand::Const(IrConst::I64(0)), head_label));
                incoming.push((Operand::Value(result), b2_label));
            }
            (join, Terminator::Branch(join))
        } else {
            let miss = cluster_next
                .or(chain_exit)
                .expect("mode A cluster has a miss target");
            if let Instruction::Phi { incoming, .. } =
                &mut func.blocks[join_idx].instructions[phi_pos]
            {
                incoming.push((Operand::Const(IrConst::I64(1)), b2_label));
            }
            (
                miss,
                Terminator::CondBranch {
                    cond: Operand::Value(bit_v),
                    true_label: join,
                    false_label: miss,
                },
            )
        };

        let keep = chain[head].prelude_len;
        let mut head_insts: Vec<Instruction> =
            func.blocks[head_bi].instructions[..keep].to_vec();
        head_insts.extend(b1);
        func.blocks[head_bi].instructions = head_insts;
        func.blocks[head_bi].source_spans.clear();
        func.blocks[head_bi].terminator = Terminator::CondBranch {
            cond: Operand::Value(out_v),
            true_label: guard_miss,
            false_label: b2_label,
        };
        // Appending keeps every recorded block index valid (IR block order
        // carries no fallthrough semantics; block_layout reorders later).
        func.blocks.push(BasicBlock {
            label: b2_label,
            instructions: b2,
            terminator: b2_term,
            source_spans: vec![],
        });
        func.next_label = next_label;
        func.next_value_id = next_id;
        changes += 1;
        // One transformation per call: the appended block invalidated the
        // label map. The driver's fixpoint loop re-enters for more chains.
        return changes;
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eq_block(label: u32, x: Value, con: i8, join: u32, miss: u32, d: Value) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: vec![Instruction::Cmp {
                dest: d,
                op: IrCmpOp::Eq,
                lhs: Operand::Value(x),
                rhs: Operand::Const(IrConst::I8(con)),
                ty: IrType::U8,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(d),
                true_label: BlockId(join),
                false_label: BlockId(miss),
            },
            source_spans: vec![],
        }
    }

    fn skip_block(label: u32, x: Value, con: i8, join: u32, miss: u32, d: Value) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: vec![Instruction::Cmp {
                dest: d,
                op: IrCmpOp::Uge,
                lhs: Operand::Value(x),
                rhs: Operand::Const(IrConst::I8(con)),
                ty: IrType::U8,
            }],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(d),
                true_label: BlockId(join),
                false_label: BlockId(miss),
            },
            source_spans: vec![],
        }
    }

    fn tail_block(label: u32, x: Value, con: i8, join: u32, d: Value) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: vec![Instruction::Cmp {
                dest: d,
                op: IrCmpOp::Eq,
                lhs: Operand::Value(x),
                rhs: Operand::Const(IrConst::I8(con)),
                ty: IrType::U8,
            }],
            terminator: Terminator::Branch(BlockId(join)),
            source_spans: vec![],
        }
    }

    fn range_block(
        label: u32,
        x: Value,
        lo: i32,
        span: i32,
        join: u32,
        miss: u32,
        base: u32,
    ) -> BasicBlock {
        let c = Value(base);
        let s = Value(base + 1);
        let b = Value(base + 2);
        let w = Value(base + 3);
        BasicBlock {
            label: BlockId(label),
            instructions: vec![
                Instruction::Cast {
                    dest: c,
                    src: Operand::Value(x),
                    from_ty: IrType::U8,
                    to_ty: IrType::I32,
                },
                Instruction::BinOp {
                    dest: s,
                    op: IrBinOp::Sub,
                    lhs: Operand::Value(c),
                    rhs: Operand::Const(IrConst::I32(lo)),
                    ty: IrType::I32,
                },
                Instruction::Cmp {
                    dest: b,
                    op: IrCmpOp::Ule,
                    lhs: Operand::Value(s),
                    rhs: Operand::Const(IrConst::I32(span)),
                    ty: IrType::U32,
                },
                Instruction::Cast {
                    dest: w,
                    src: Operand::Value(b),
                    from_ty: IrType::I8,
                    to_ty: IrType::I64,
                },
            ],
            terminator: Terminator::CondBranch {
                cond: Operand::Value(w),
                true_label: BlockId(join),
                false_label: BlockId(miss),
            },
            source_spans: vec![],
        }
    }

    fn join_block(label: u32, phi: Value, incoming: Vec<(Operand, BlockId)>) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: vec![Instruction::Phi { dest: phi, ty: IrType::I64, incoming }],
            terminator: Terminator::Return(Some(Operand::Value(phi))),
            source_spans: vec![],
        }
    }

    fn exit_block(label: u32) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: vec![],
        }
    }

    /// Interpret the rewritten function for one byte value: returns the
    /// phi result (0/1). Follows CondBranch/Branch chains from block 0.
    fn eval(f: &IrFunction, x: u8) -> i64 {
        let idx: FxHashMap<u32, usize> =
            f.blocks.iter().enumerate().map(|(i, b)| (b.label.0, i)).collect();
        let mut vals: FxHashMap<u32, i64> = FxHashMap::default();
        vals.insert(1, x as i64); // root value id used by the test builders
        let mut cur = 0usize;
        let mut prev_label: Option<BlockId> = None;
        for _ in 0..64 {
            let block = &f.blocks[cur];
            for inst in &block.instructions {
                match inst {
                    Instruction::Cast { dest, src: Operand::Value(s), .. } => {
                        let v = *vals.get(&s.0).unwrap();
                        vals.insert(dest.0, v);
                    }
                    Instruction::BinOp { dest, op, lhs, rhs, .. } => {
                        let l = match lhs {
                            Operand::Value(v) => *vals.get(&v.0).unwrap(),
                            Operand::Const(c) => c.to_i64().unwrap(),
                        };
                        let r = match rhs {
                            Operand::Value(v) => *vals.get(&v.0).unwrap(),
                            Operand::Const(c) => c.to_i64().unwrap(),
                        };
                        let res = match op {
                            IrBinOp::Sub => (l as u32).wrapping_sub(r as u32) as i64,
                            IrBinOp::And => l & r,
                            IrBinOp::BitTest => (l >> ((r as u64) % 64)) & 1,
                            _ => panic!("eval: unexpected op"),
                        };
                        vals.insert(dest.0, res);
                    }
                    Instruction::Cmp { dest, op, lhs, rhs, .. } => {
                        let l = match lhs {
                            Operand::Value(v) => *vals.get(&v.0).unwrap(),
                            Operand::Const(c) => c.to_i64().unwrap(),
                        } as u32 as u64;
                        let r = match rhs {
                            Operand::Value(v) => *vals.get(&v.0).unwrap(),
                            Operand::Const(c) => c.to_i64().unwrap(),
                        } as u32 as u64;
                        // Byte compares in the fixtures: compare in u8 domain.
                        let (l, r) = match op {
                            IrCmpOp::Eq | IrCmpOp::Uge => (l as u8 as u64, r as u8 as u64),
                            _ => (l, r),
                        };
                        let res = match op {
                            IrCmpOp::Eq => l == r,
                            IrCmpOp::Ule => l <= r,
                            IrCmpOp::Ugt => l > r,
                            IrCmpOp::Uge => l >= r,
                            _ => panic!("eval: unexpected cmp"),
                        };
                        vals.insert(dest.0, res as i64);
                    }
                    Instruction::Phi { dest, incoming, .. } => {
                        let pl = prev_label.expect("phi needs a predecessor");
                        let (op, _) = incoming
                            .iter()
                            .find(|(_, b)| *b == pl)
                            .expect("phi entry for predecessor");
                        let v = match op {
                            Operand::Value(v) => *vals.get(&v.0).unwrap(),
                            Operand::Const(c) => c.to_i64().unwrap(),
                        };
                        vals.insert(dest.0, v);
                    }
                    _ => panic!("eval: unexpected instruction"),
                }
            }
            match &block.terminator {
                Terminator::Return(Some(Operand::Value(v))) => {
                    return *vals.get(&v.0).unwrap();
                }
                Terminator::Return(_) => return 0,
                Terminator::Branch(t) => {
                    prev_label = Some(block.label);
                    cur = idx[&t.0];
                }
                Terminator::CondBranch { cond, true_label, false_label } => {
                    let c = match cond {
                        Operand::Value(v) => *vals.get(&v.0).unwrap(),
                        Operand::Const(c) => c.to_i64().unwrap(),
                    };
                    prev_label = Some(block.label);
                    cur = idx[&if c != 0 { true_label.0 } else { false_label.0 }];
                }
                _ => panic!("eval: unexpected terminator"),
            }
        }
        panic!("eval: no exit in 64 steps");
    }

    /// The full Expat classifier chain (post range_fold shape) with the
    /// `>= 0xc2` skip in the middle; reference semantics for exhaustive
    /// before/after comparison.
    fn expat_chain() -> IrFunction {
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            range_block(0, x, 97, 25, 90, 1, 60), // a-z
            range_block(1, x, 65, 25, 90, 2, 66), // A-Z
            eq_block(2, x, 95, 90, 3, Value(70)), // _
            eq_block(3, x, 58, 90, 4, Value(71)), // :
            skip_block(4, x, -62i8, 90, 5, Value(72)), // >= 0xc2
            range_block(5, x, 48, 9, 90, 6, 74),  // 0-9
            eq_block(6, x, 45, 90, 7, Value(78)), // -
            tail_block(7, x, 46, 90, Value(79)),  // .
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Const(IrConst::I64(1)), BlockId(1)),
                    (Operand::Const(IrConst::I64(1)), BlockId(2)),
                    (Operand::Const(IrConst::I64(1)), BlockId(3)),
                    (Operand::Const(IrConst::I64(1)), BlockId(4)),
                    (Operand::Const(IrConst::I64(1)), BlockId(5)),
                    (Operand::Const(IrConst::I64(1)), BlockId(6)),
                    (Operand::Value(Value(79)), BlockId(7)),
                ],
            ),
        ];
        f.next_value_id = 100;
        f
    }

    fn reference(x: u8) -> i64 {
        let c = x;
        ((c >= b'a' && c <= b'z')
            || (c >= b'A' && c <= b'Z')
            || c == b'_'
            || c == b':'
            || c >= 0xc2
            || (c >= b'0' && c <= b'9')
            || c == b'-'
            || c == b'.') as i64
    }

    #[test]
    fn expat_chain_folds_to_fixpoint_and_stays_exact() {
        let mut f = expat_chain();
        // Run to fixpoint like the driver does.
        let mut total = 0;
        for _ in 0..8 {
            let n = run_function(&mut f);
            if n == 0 {
                break;
            }
            total += n;
        }
        assert!(total >= 2, "case-fold + cluster expected, got {total} changes");
        // Case-fold happened: some block computes x & ~32.
        assert!(
            f.blocks.iter().any(|b| b.instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp { op: IrBinOp::And, rhs: Operand::Const(IrConst::I32(c)), .. }
                    if *c == !32
            ))),
            "expected the ASCII case-fold And"
        );
        // ONE BitTest cluster (not two): count BitTest instructions.
        let bt_count = f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| matches!(i, Instruction::BinOp { op: IrBinOp::BitTest, .. }))
            .count();
        assert_eq!(bt_count, 1, "skip-tolerant collection must build ONE cluster");
        // Exhaustive 0..255 equivalence.
        for x in 0..=255u8 {
            assert_eq!(eval(&f, x), reference(x), "mismatch at byte {x}");
        }
    }

    #[test]
    fn case_fold_requires_bit5_clear() {
        // Ranges [40,72] and [72,104] do NOT satisfy the fold (bit 5 not
        // clear across [40,72]); nothing may fold them into an And test.
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            range_block(0, x, 40, 32, 90, 1, 60),
            range_block(1, x, 72, 32, 90, 2, 66),
            exit_block(2),
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Const(IrConst::I64(1)), BlockId(1)),
                ],
            ),
        ];
        f.next_value_id = 100;
        run_function(&mut f);
        assert!(
            !f.blocks.iter().any(|b| b.instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp { op: IrBinOp::And, .. }
            ))),
            "case-fold must not fire without the bit-5 invariant"
        );
    }

    #[test]
    fn cluster_collects_across_skip() {
        // {95, 58} SKIP {45} — three eq members separated by a skip fold
        // into one cluster; the skip survives and chains after the guard.
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            eq_block(0, x, 95, 90, 1, Value(70)),
            eq_block(1, x, 58, 90, 2, Value(71)),
            skip_block(2, x, -62i8, 90, 3, Value(72)),
            eq_block(3, x, 45, 90, 4, Value(73)),
            exit_block(4),
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Const(IrConst::I64(1)), BlockId(1)),
                    (Operand::Const(IrConst::I64(1)), BlockId(2)),
                    (Operand::Const(IrConst::I64(1)), BlockId(3)),
                ],
            ),
        ];
        f.next_value_id = 100;
        assert_eq!(run_function(&mut f), 1);
        let bt_count = f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| matches!(i, Instruction::BinOp { op: IrBinOp::BitTest, .. }))
            .count();
        assert_eq!(bt_count, 1);
        for x in 0..=255u8 {
            let want = (x == 95 || x == 58 || x >= 0xc2 || x == 45) as i64;
            assert_eq!(eval(&f, x), want, "mismatch at byte {x}");
        }
    }

    #[test]
    fn two_members_not_folded() {
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            eq_block(0, x, 45, 90, 1, Value(70)),
            tail_block(1, x, 46, 90, Value(71)),
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Value(Value(71)), BlockId(1)),
                ],
            ),
        ];
        f.next_value_id = 100;
        assert_eq!(run_function(&mut f), 0);
    }

    #[test]
    fn window_selection_drops_outlier() {
        // {0, 5, 9, 200}: the window keeps {0,5,9} and leaves 200 as a
        // surviving eq test (as the converted value tail).
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            eq_block(0, x, 0, 90, 1, Value(70)),
            eq_block(1, x, 5, 90, 2, Value(71)),
            eq_block(2, x, 9, 90, 3, Value(72)),
            tail_block(3, x, -56i8, 90, Value(73)), // 200
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Const(IrConst::I64(1)), BlockId(1)),
                    (Operand::Const(IrConst::I64(1)), BlockId(2)),
                    (Operand::Value(Value(73)), BlockId(3)),
                ],
            ),
        ];
        f.next_value_id = 100;
        assert_eq!(run_function(&mut f), 1);
        for x in 0..=255u8 {
            let want = (x == 0 || x == 5 || x == 9 || x == 200) as i64;
            assert_eq!(eval(&f, x), want, "mismatch at byte {x}");
        }
    }

    #[test]
    fn mid_chain_extra_pred_stops_chain() {
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            eq_block(0, x, 45, 90, 1, Value(70)),
            eq_block(1, x, 46, 90, 2, Value(71)),
            tail_block(2, x, 58, 90, Value(72)),
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Const(IrConst::I64(1)), BlockId(1)),
                    (Operand::Value(Value(72)), BlockId(2)),
                ],
            ),
            BasicBlock {
                label: BlockId(5),
                instructions: vec![],
                terminator: Terminator::Branch(BlockId(1)),
                source_spans: vec![],
            },
        ];
        f.next_value_id = 100;
        assert_eq!(run_function(&mut f), 0);
    }

    #[test]
    fn tail_cmp_with_extra_use_not_absorbed() {
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            eq_block(0, x, 45, 90, 1, Value(70)),
            eq_block(1, x, 46, 90, 2, Value(71)),
            tail_block(2, x, 58, 90, Value(72)),
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Const(IrConst::I64(1)), BlockId(1)),
                    (Operand::Value(Value(72)), BlockId(2)),
                ],
            ),
            BasicBlock {
                label: BlockId(6),
                instructions: vec![],
                terminator: Terminator::Return(Some(Operand::Value(Value(72)))),
                source_spans: vec![],
            },
        ];
        f.next_value_id = 100;
        assert_eq!(run_function(&mut f), 0);
    }

    #[test]
    fn unsigned_constants_normalize() {
        let x = Value(1);
        let phi = Value(50);
        let mut f = IrFunction::new("f".into(), IrType::I64, vec![], false);
        f.blocks = vec![
            eq_block(0, x, -62i8, 90, 1, Value(70)), // 194
            eq_block(1, x, -61i8, 90, 2, Value(71)), // 195
            tail_block(2, x, -32i8, 90, Value(72)),  // 224
            join_block(
                90,
                phi,
                vec![
                    (Operand::Const(IrConst::I64(1)), BlockId(0)),
                    (Operand::Const(IrConst::I64(1)), BlockId(1)),
                    (Operand::Value(Value(72)), BlockId(2)),
                ],
            ),
        ];
        f.next_value_id = 100;
        assert_eq!(run_function(&mut f), 1);
        for x in 0..=255u8 {
            let want = (x == 194 || x == 195 || x == 224) as i64;
            assert_eq!(eval(&f, x), want, "mismatch at byte {x}");
        }
    }
}
