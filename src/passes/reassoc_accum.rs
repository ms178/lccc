//! Loop-carried accumulator reassociation (ICC's Adler-32 chain break).
//!
//! `sum1 += b[i]; sum2 += sum1;` forms a serial chain: every `sum2 += sum1`
//! step depends on the matching `sum1 += b[i]` step, so the two loop-carried
//! chains cannot overlap and the loop is latency-bound (one add per byte on
//! the critical path, plus the load latency).
//!
//! For UNSIGNED accumulation the additions are associative modulo 2^n (no
//! overflow UB), so an N-step unroll has the exact closed form
//!
//! ```text
//!     sum1' = sum1 + Σᵢ b[i]
//!     sum2' = sum2 + N·sum1 + Σᵢ (N−i)·b[i]
//! ```
//!
//! which recomputes `sum2'` directly from the OLD `sum1` and the byte loads —
//! breaking the serial dependency between the two chains (ICC's 4-register
//! rotation for zlib's Adler-32). The weighted byte terms are independent, so
//! the hardware pipelines them; the only remaining serial links are the two
//! loop-carried phi values.
//!
//! Soundness: only U32/U64 `Add` chains are transformed (signed addition would
//! be UB on overflow); the adler chain is left untouched (it is already the
//! optimal linear accumulation); every coefficient multiply is by a small
//! constant that the backend strength-reduces (imul/lea/shift).
//!
//! Pass name for CCC_DISABLE_PASSES: "reassoc_accum".

use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

/// Whether `ty` is an unsigned integer type eligible for reassociation.
fn is_unsigned_add_ty(ty: IrType) -> bool {
    matches!(ty, IrType::U32 | IrType::U64)
}

fn int_const(ty: IrType, v: i64) -> IrConst {
    if ty == IrType::U32 {
        IrConst::I32(v as i32)
    } else {
        IrConst::I64(v)
    }
}

/// Call `f` for every Value used as an Add operand in `inst`.
fn for_each_add_operand(inst: &Instruction, mut f: impl FnMut(u32)) {
    if let Instruction::BinOp {
        op: IrBinOp::Add,
        lhs,
        rhs,
        ..
    } = inst
    {
        if let Operand::Value(l) = lhs {
            f(l.0);
        }
        if let Operand::Value(r) = rhs {
            f(r.0);
        }
    }
}

/// Run the reassociation over one function. Returns the number of rewrites.
pub(crate) fn run_function(func: &mut IrFunction) -> usize {
    // Function-wide use map: value -> (block_idx, inst_idx) of every Add that
    // consumes it. The accumulator chains live in the loop BODY block while
    // the phis live in the header, so the walk must cross blocks.
    let mut uses: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            for_each_add_operand(inst, |v| {
                uses.entry(v).or_default().push((bi, ii));
            });
        }
    }

    // Phi destinations and every phi incoming value (function-wide).
    let mut phi_dests: Vec<Value> = Vec::new();
    let mut phi_incoming: Vec<u32> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                phi_dests.push(*dest);
                for (op, _) in incoming {
                    if let Operand::Value(v) = op {
                        phi_incoming.push(v.0);
                    }
                }
            }
        }
    }
    if phi_dests.is_empty() {
        return 0;
    }

    let mut changes = 0usize;
    for &adler0 in &phi_dests {
        // Walk the adler chain forward from the phi's first use:
        // adler_{i+1} = Add(adler_i, c_i) with the accumulator as LHS and a
        // non-phi byte operand as RHS. (The sum2 add `Add(sum2, adler)` has
        // the current value as RHS, which the LHS requirement skips.)
        let mut chain: Vec<(Value, Value, IrType, usize)> = Vec::new();
        let mut cur = adler0;
        loop {
            let mut next: Option<(Value, Value, IrType, usize)> = None;
            if let Some(ulist) = uses.get(&cur.0) {
                for &(bi, ii) in ulist {
                    let inst = &func.blocks[bi].instructions[ii];
                    if let Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty,
                    } = inst
                    {
                        if is_unsigned_add_ty(*ty) {
                            if let (Operand::Value(l), Operand::Value(r)) = (lhs, rhs) {
                                if l.0 == cur.0 && !phi_dests.iter().any(|p| p.0 == r.0) {
                                    next = Some((*dest, *r, *ty, bi));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            match next {
                Some((d, c, t, b)) => {
                    chain.push((d, c, t, b));
                    cur = d;
                    if chain.len() > 64 {
                        break;
                    }
                }
                None => break,
            }
        }
        if chain.len() < 4 {
            continue;
        }
        let ty = chain[0].2;
        let n = chain.len();
        let adler_n = chain[n - 1].0;
        let adler1 = chain[0].0;
        // All chain instructions must live in ONE block (the rewrite splices
        // that block). True of the source-unrolled adler body.
        let body_block = chain[0].3;
        if chain.iter().any(|&(_, _, _, b)| b != body_block) {
            continue;
        }

        // Find sum2_0: the Add consuming adler1 (as RHS) whose LHS is a phi.
        let mut sum2_0: Option<Value> = None;
        'outer: for &(bi, ii) in uses.get(&adler1.0).into_iter().flatten() {
            let inst = &func.blocks[bi].instructions[ii];
            if let Instruction::BinOp {
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: t,
                ..
            } = inst
            {
                if *t != ty {
                    continue;
                }
                if let (Operand::Value(l), Operand::Value(r)) = (lhs, rhs) {
                    if r.0 == adler1.0 && phi_dests.iter().any(|p| p.0 == l.0) {
                        sum2_0 = Some(*l);
                        break 'outer;
                    }
                }
            }
        }
        let sum2_0 = match sum2_0 {
            Some(v) => v,
            None => continue,
        };

        // Verify the full sum2 chain: sum2_{i+1} = Add(sum2_i, adler_{i+1}).
        let mut sum2_cur = sum2_0;
        let mut sum2_n: Option<Value> = None;
        let mut sum2_ok = true;
        for &(adler_next, _, _, _) in &chain {
            let mut found = None;
            if let Some(ulist) = uses.get(&sum2_cur.0) {
                for &(bi, ii) in ulist {
                    let inst = &func.blocks[bi].instructions[ii];
                    if let Instruction::BinOp {
                        dest,
                        op: IrBinOp::Add,
                        lhs,
                        rhs,
                        ty: t,
                    } = inst
                    {
                        if *t != ty {
                            continue;
                        }
                        if let (Operand::Value(l), Operand::Value(r)) = (lhs, rhs) {
                            if l.0 == sum2_cur.0 && r.0 == adler_next.0 {
                                found = Some(*dest);
                                break;
                            }
                        }
                    }
                }
            }
            match found {
                Some(d) => {
                    sum2_cur = d;
                    sum2_n = Some(d);
                }
                None => {
                    sum2_ok = false;
                    break;
                }
            }
        }
        if !sum2_ok {
            continue;
        }
        let sum2_n = match sum2_n {
            Some(v) => v,
            None => continue,
        };

        // adler_N and sum2_N must be phi incoming values (loop-carried).
        if !phi_incoming.contains(&adler_n.0) || !phi_incoming.contains(&sum2_n.0) {
            continue;
        }

        // ── Cost model (session 30, measured ──────────────────────────────
        //
        // `sum2' = sum2 + N*adler + Σ (N-i)*b[i]` replaces N dependent adds
        // with 2N+2 mostly-independent ops (one shift/mul and one add per
        // term) and, worse for a backend without instruction scheduling,
        // keeps all N byte terms AND all N weighted terms live to the splice
        // point. It wins only when the loop-carried recurrence — not issue
        // throughput — is the binding constraint.
        //
        // Both accumulator phis advance by a single 1-cycle add per
        // iteration, so the inter-iteration critical path is ~2 cycles and an
        // out-of-order core overlaps iterations freely: with a 4-wide
        // machine any body of >= ~8 ops is already throughput-bound. The pass
        // requires N >= 4, which guarantees a body of at least 4 loads +
        // 8 adds — i.e. exactly the throughput-bound regime.
        //
        // Measured on the source-unrolled zlib DO8 loop (1 MiB x 30, -O2,
        // see k_adler32_do8 / scripts/bench_kernels.py):
        //
        //     N     insns on/off     runtime on/off
        //     4      75 / 58          21.7 / 10.2 ms
        //     8     106 / 70          18.7 / 11.1 ms
        //    16     169 / 94          20.3 / 10.5 ms
        //
        // The closed form is a uniform ~1.9x slowdown and +30..80% code size
        // on its own flagship pattern, so the rewrite is gated on a real
        // cycle estimate instead of being applied unconditionally. Set
        // CCC_REASSOC_ACCUM_FORCE=1 to bypass the model (research / targets
        // where the recurrence, not throughput, dominates).
        if !force_reassoc_accum() {
            // Throughput bound from the CPU tuning model: the rename width
            // of the tuned core (4 on Generic/SNB..SKX, 5 on ICL, 6 on
            // Golden/Raptor Cove and Zen1..4, 8 on Lion Cove / Zen5); the
            // i686 target keeps its historical 3-wide estimate.
            let width: usize = if crate::common::types::target_is_32bit() {
                3
            } else {
                crate::backend::x86::cpu_model::active().issue_width()
            };
            let body_ops = func.blocks[body_block].instructions.len();
            // N removed adds, N*(shift|mul) + N adds + 2 spliced back in.
            let ops_after = body_ops + n + 2;
            // Loop-carried critical path: both phis are one 1-cycle add.
            let recurrence = 2usize;
            let cycles_before = recurrence.max(body_ops / width);
            let cycles_after = recurrence.max(ops_after / width);
            if cycles_after >= cycles_before {
                continue;
            }
            // Peak register pressure: N byte terms + N weighted terms +
            // the s1/s2/pointer/counter quartet must coexist at the splice.
            let usable = if crate::common::types::target_is_32bit() {
                6usize
            } else {
                12usize
            };
            if 2 * n + 4 > usable {
                continue;
            }
        }

        if rewrite_sum2_chain(func, body_block, &chain, adler0, sum2_0, sum2_n, ty) {
            changes += 1;
        }
    }
    changes
}

/// Replace the N-step sum2 chain with `sum2_0 + N*adler0 + Σ (N-i)*c_i`,
/// reusing `sum2_n`'s value id for the final result.
fn rewrite_sum2_chain(
    func: &mut IrFunction,
    bi: usize,
    chain: &[(Value, Value, IrType, usize)],
    adler0: Value,
    sum2_0: Value,
    sum2_n: Value,
    ty: IrType,
) -> bool {
    let n = chain.len() as i64;
    let mut next_id = func.next_value_id;
    if next_id == 0 {
        next_id = func.max_value_id() + 1;
    }
    let mut fresh = |next_id: &mut u32| -> Value {
        let v = Value(*next_id);
        *next_id += 1;
        v
    };

    // Closed-form intermediates (computed BEFORE the final add).
    let mut seq: Vec<Instruction> = Vec::new();

    // acc = N * adler0  (shift when N is a power of two, else multiply).
    let acc0 = fresh(&mut next_id);
    let (op0, rhs0) = if n > 0 && (n & (n - 1)) == 0 {
        (IrBinOp::Shl, int_const(ty, n.trailing_zeros() as i64))
    } else {
        (IrBinOp::Mul, int_const(ty, n))
    };
    seq.push(Instruction::BinOp {
        dest: acc0,
        op: op0,
        lhs: Operand::Value(adler0),
        rhs: Operand::Const(rhs0),
        ty,
    });
    let mut acc = acc0;

    // For each byte: acc += (N - i) * c_i.
    for (i, &(_, c_i, _, _)) in chain.iter().enumerate() {
        let k = n - i as i64; // N, N-1, ..., 1
        let w = fresh(&mut next_id);
        let (op, rhs) = if k > 0 && (k & (k - 1)) == 0 {
            (IrBinOp::Shl, int_const(ty, k.trailing_zeros() as i64))
        } else {
            (IrBinOp::Mul, int_const(ty, k))
        };
        seq.push(Instruction::BinOp {
            dest: w,
            op,
            lhs: Operand::Value(c_i),
            rhs: Operand::Const(rhs),
            ty,
        });
        let acc2 = fresh(&mut next_id);
        seq.push(Instruction::BinOp {
            dest: acc2,
            op: IrBinOp::Add,
            lhs: Operand::Value(acc),
            rhs: Operand::Value(w),
            ty,
        });
        acc = acc2;
    }

    // Final: sum2_n = sum2_0 + acc (reuses sum2_n's dest id).
    seq.push(Instruction::BinOp {
        dest: sum2_n,
        op: IrBinOp::Add,
        lhs: Operand::Value(sum2_0),
        rhs: Operand::Value(acc),
        ty,
    });

    // Identify the positions of the old sum2-chain adds (walking the same
    // chain), so we can drop them and splice the closed form in at the final
    // sum2 add's position (all its inputs are defined by then).
    let block = &func.blocks[bi];
    let mut old_indices: Vec<usize> = Vec::new();
    {
        let mut cur = sum2_0;
        for &(adler_next, _, _, _) in chain {
            let mut found = None;
            for (idx, inst) in block.instructions.iter().enumerate() {
                if let Instruction::BinOp {
                    dest,
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                } = inst
                {
                    if let (Operand::Value(l), Operand::Value(r)) = (lhs, rhs) {
                        if l.0 == cur.0 && r.0 == adler_next.0 {
                            found = Some((idx, *dest));
                            break;
                        }
                    }
                }
            }
            match found {
                Some((idx, d)) => {
                    old_indices.push(idx);
                    cur = d;
                }
                None => return false,
            }
        }
    }
    if old_indices.len() != chain.len() {
        return false;
    }

    let block = &mut func.blocks[bi];
    let mut new_insts: Vec<Instruction> = Vec::with_capacity(block.instructions.len() + seq.len());
    for (idx, inst) in block.instructions.drain(..).enumerate() {
        if old_indices.contains(&idx) {
            if inst.dest().is_some_and(|d| d.0 == sum2_n.0) {
                new_insts.extend(seq.iter().cloned());
            }
            // else: an intermediate sum2 add — drop it (now dead).
        } else {
            new_insts.push(inst);
        }
    }
    block.instructions = new_insts;
    func.next_value_id = next_id;
    true
}

/// `CCC_REASSOC_ACCUM_FORCE=1` bypasses the cost model above. Kept so the
/// transform stays reachable for targets (in-order cores, long-latency
/// accumulator operations) where the throughput/latency trade-off inverts.
fn force_reassoc_accum() -> bool {
    std::env::var_os("CCC_REASSOC_ACCUM_FORCE").is_some()
}
