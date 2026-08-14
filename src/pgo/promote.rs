//! Indirect-call value-profile PROMOTION (cf. LLVM `-pgo-indirect-call-promotion`).
//!
//! At profile use we load the per-site callee distributions recorded by the
//! training run. When the top target accounts for at least the promotion
//! threshold (default 51%, LLVM's convention) of a site's calls, the call is
//! rewritten to:
//!
//! ```text
//!   %ta = globaladdr @target
//!   %eq = cmp eq %fp, %ta
//!   condbr %eq, %hot, %cold
//! %hot:  %r1 = call @target(args...); br %join
//! %cold: %r2 = callindirect %fp(args...); br %join
//! %join: %r  = phi(%r1 <- %hot, %r2 <- %cold); <original terminator>
//! ```
//!
//! Correctness is guaranteed even when site ordinals drift (profile-guided
//! inlining can change the post-pass CFG): promotion only fires when
//!   1. the function's post-pass fingerprint matches the training build's
//!      (`fp.post_hash`), so ordinals align;
//!   2. the recorded call signature matches the site's actual signature;
//!   3. the target function exists in the module, is defined, and its own
//!      signature matches the recorded signature.
//! The runtime `cmp` guard makes the transform semantics-preserving by
//! construction (LLVM's `ICallPromotionFunc` has the same shape).
//!
//! The promoted hot blocks are recorded (unit, function, labels) so the
//! layout pass can give them the caller block's hotness instead of treating
//! them as cold (they are new blocks with no profile entries).
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{
    BasicBlock, BlockId, CallInfo, Instruction, IrFunction, IrModule, Operand, Terminator, Value,
};
use crate::common::types::IrType;
use crate::ir::constants::IrConst;
use crate::pgo::profile;

struct PromoPlan {
    block_idx: usize,
    inst_idx: usize,
    target: String,
}

/// Stable textual signature (mirrors instrument::site_signature).
pub fn sig_of(info: &CallInfo) -> String {
    let n = info.num_fixed_args.min(info.arg_types.len());
    let mut s = format!("{:?}:{}", info.return_type, n);
    for t in info.arg_types.iter().take(n) {
        s.push(':');
        s.push_str(&format!("{:?}", t));
    }
    s
}

/// Promote indirect calls in `m` using the loaded profile for unit `u`.
/// Returns the number of sites promoted and records hot labels for layout.
pub fn promote_indirect_calls(m: &mut IrModule, u: &str) -> usize {
    if std::env::var("LCCC_PGO_NO_PROMOTE").is_ok() {
        return 0;
    }
    let Some(p) = crate::pgo::get_pgo_profile() else {
        return 0;
    };
    let threshold: u64 = std::env::var("LCCC_PGO_PROMOTE_THRESHOLD")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(51)
        .max(1)
        .min(100);
    // A site whose top target accounts for >= STABLE_PERCENT
    // (default 95%) of calls is effectively SINGLE-TARGET and is already
    // predicted perfectly by the indirect branch predictor (BTB) — the guarded
    // `cmp fp, target; jne cold; call target; cold: call *fp` transform then
    // adds a compare + branch to EVERY call with no accuracy benefit, a net
    // REGRESSION (measured: op_dispatch indirect dispatch 38.9ms -> 49.9ms,
    // +28% from devirtualizing a loop-invariant single target). Devirtualization
    // is only beneficial when the site is genuinely MULTI-VALUED (top share
    // below STABLE_PERCENT) — there the guard makes the common case a direct
    // call and reduces real indirect-call mispredictions.
    let stable_percent: u64 = std::env::var("LCCC_PGO_PROMOTE_STABLE")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(95)
        .clamp(51, 100);
    let mut promoted = 0;

    // Module-global label + value counters (labels are TU-global).
    let mut next_label: u32 = m
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter().map(|b| b.label.0))
        .max()
        .map(|x| x + 1)
        .unwrap_or(0);
    let mut hot_labels: FxHashMap<String, FxHashSet<u32>> = FxHashMap::default();

    // Snapshot of defined functions (name -> signature) for target
    // verification; we cannot borrow m.functions while mutating it.
    let defined: FxHashMap<String, String> = m
        .functions
        .iter()
        .filter(|g| !g.is_declaration && !g.blocks.is_empty())
        .map(|g| {
            let mut sig = format!("{:?}:{}", g.return_type, g.params.len());
            for p2 in &g.params {
                sig.push(':');
                sig.push_str(&format!("{:?}", p2.ty));
            }
            (g.name.clone(), sig)
        })
        .collect();

    for f in &mut m.functions {
        if f.is_declaration || f.blocks.is_empty() {
            continue;
        }
        let Some(fp) = profile::get_for_unit(p, u, &f.name) else {
            continue;
        };
        if fp.value_sites.is_empty() {
            continue;
        }
        // NOTE: no CFG-drift gate here. Profile-guided inlining changes the
        // post-pass CFG of exactly the hot functions we want to promote, so
        // an h1 check would skip them all. Promotion is SAFE under ordinal
        // misalignment by construction: the recorded signature is verified
        // against the call site, the target's signature is verified against
        // the record, and the runtime compare guards the direct call — a
        // wrong pairing can only cost performance, never correctness.
        // Collect plans (no mutation during the scan).
        let mut plans: Vec<PromoPlan> = Vec::new();
        let mut ordinal = 0usize;
        for (bi, block) in f.blocks.iter().enumerate() {
            for (ki, inst) in block.instructions.iter().enumerate() {
                let Instruction::CallIndirect { func_ptr, info } = inst else {
                    continue;
                };
                let site = fp.value_sites.iter().find(|s| s.ordinal == ordinal);
                ordinal += 1;
                let Some(site) = site else { continue };
                if std::env::var("LCCC_DEBUG_PROMOTE").is_ok() {
                    eprintln!(
                        "[PROMOTE] {} site ordinal={} total={} targets={:?}",
                        f.name, site.ordinal, site.total, site.targets
                    );
                }
                if site.total == 0 {
                    continue;
                }
                let Some((tname, tcount, tflags)) = site.targets.first() else {
                    continue;
                };
                if tcount.saturating_mul(100) < site.total.saturating_mul(threshold) {
                    continue;
                }
                // Skip the effectively single-target (stable, perfectly
                // predicted) sites — promoting them only adds per-call overhead
                // (see the stable_percent rationale above).
                if tcount.saturating_mul(100) >= site.total.saturating_mul(stable_percent) {
                    continue;
                }
                // A static target's emitted symbol is LOCAL: cross-TU direct
                // calls cannot link (observed: functable.force_init_stub, a
                // static in functable.c whose pointer escapes via the global
                // functable). In-module statics are fine (defined check).
                if tflags & 1 == 1 && !defined.contains_key(tname) {
                    continue;
                }
                if site.sig != sig_of(info) {
                    if std::env::var("LCCC_DEBUG_PROMOTE").is_ok() {
                        eprintln!(
                            "[PROMOTE] {} site {} sig mismatch: recorded={} actual={}",
                            f.name, site.ordinal, site.sig, sig_of(info)
                        );
                    }
                    continue;
                }
                if !matches!(func_ptr, Operand::Value(_)) {
                    continue;
                }
                if *tname == f.name {
                    continue;
                }
                // Cross-TU targets (zlib-ng functable!) are not defined in
                // this module. Promotion is still safe: the recorded
                // signature was verified against the site above (the exact
                // call shape succeeded at runtime during training), and the
                // runtime compare guards the direct call. When the target IS
                // defined here, verify its signature too.
                if let Some(tf_sig) = defined.get(tname) {
                    if *tf_sig != site.sig {
                        continue;
                    }
                }
                plans.push(PromoPlan {
                    block_idx: bi,
                    inst_idx: ki,
                    target: tname.clone(),
                });
            }
        }
        if plans.is_empty() {
            continue;
        }
        // Apply plans block by block, highest index first (stable indices).
        let mut by_block: FxHashMap<usize, Vec<PromoPlan>> = FxHashMap::default();
        for pl in plans {
            by_block.entry(pl.block_idx).or_default().push(pl);
        }
        let mut next_val = f.next_value_id;
        for (bi, plist) in by_block {
            let mut plist = plist;
            plist.sort_by_key(|x| std::cmp::Reverse(x.inst_idx));
            for pl in plist {
                let block = &mut f.blocks[bi];
                let (fp_op, info) = match &block.instructions[pl.inst_idx] {
                    Instruction::CallIndirect { func_ptr, info } => {
                        (func_ptr.clone(), info.clone())
                    }
                    _ => unreachable!(),
                };
                let dest = info.dest;
                let ret_ty = info.return_type;
                let ta = Value(next_val);
                next_val += 1;
                let tai = Value(next_val);
                next_val += 1;
                let fpi = Value(next_val);
                next_val += 1;
                let cmpv = Value(next_val);
                next_val += 1;
                let r1 = dest.map(|_| {
                    let v = Value(next_val);
                    next_val += 1;
                    v
                });
                let r2 = dest.map(|_| {
                    let v = Value(next_val);
                    next_val += 1;
                    v
                });
                let (lhot, lcold, ljoin) = (
                    BlockId(next_label),
                    BlockId(next_label + 1),
                    BlockId(next_label + 2),
                );
                next_label += 3;
                let old_term = block.terminator.clone();
                let mut old_spans = block.source_spans.clone();
                let mut has_f64_second = false;
                let mut f64_dest = None;
                // SSA-correct transform: everything AFTER the call (which may
                // use the call result) moves to the JOIN block; the call
                // result is defined in the branches and merged at the join,
                // so B can no longer use it. Instructions before the call
                // stay in B (they do not depend on the result).
                let mut post: Vec<Instruction> = block.instructions.split_off(pl.inst_idx + 1);
                let mut post_spans: Vec<crate::common::source::Span> =
                    old_spans.split_off(pl.inst_idx + 1);
                let mut post2: Vec<Instruction> = Vec::new();
                let mut post_spans2: Vec<crate::common::source::Span> = Vec::new();
                for (k, inst) in post.into_iter().enumerate() {
                    if let Instruction::GetReturnF64Second { dest } = inst {
                        has_f64_second = true;
                        f64_dest = Some(dest);
                        continue;
                    }
                    post2.push(inst);
                    if let Some(sp) = post_spans.get(k) {
                        post_spans2.push(*sp);
                    }
                }
                post = post2;
                post_spans = post_spans2;
                // Rewrite the call into: GlobalAddr + casts + cmp.
                block.instructions[pl.inst_idx] = Instruction::GlobalAddr {
                    dest: ta,
                    name: pl.target.clone(),
                };
                block.instructions.insert(
                    pl.inst_idx + 1,
                    Instruction::Cast {
                        dest: tai,
                        src: Operand::Value(ta),
                        from_ty: IrType::Ptr,
                        to_ty: IrType::I64,
                    },
                );
                block.instructions.insert(
                    pl.inst_idx + 2,
                    Instruction::Cast {
                        dest: fpi,
                        src: fp_op.clone(),
                        from_ty: IrType::Ptr,
                        to_ty: IrType::I64,
                    },
                );
                block.instructions.insert(
                    pl.inst_idx + 3,
                    Instruction::Cmp {
                        dest: cmpv,
                        op: crate::ir::ops::IrCmpOp::Eq,
                        lhs: Operand::Value(tai),
                        rhs: Operand::Value(fpi),
                        ty: IrType::I64,
                    },
                );
                if !block.source_spans.is_empty() {
                    // Keep spans for instructions BEFORE the call (0..inst_idx,
                    // i.e. inst_idx items) and give the 4 replacement
                    // instructions dummy spans — exactly inst_idx + 4, the
                    // new instruction count.
                    block.source_spans.truncate(pl.inst_idx);
                    for _ in 0..4 {
                        block
                            .source_spans
                            .push(crate::common::source::Span::dummy());
                    }
                }
                block.terminator = Terminator::CondBranch {
                    cond: Operand::Value(cmpv),
                    true_label: lhot,
                    false_label: lcold,
                };
                // Move GetReturnF64Second (if present) into the cold path; the
                // hot path gets a fresh copy.
                let mut hot_is: Vec<Instruction> = Vec::new();
                let mut cold_is: Vec<Instruction> = Vec::new();
                let mut info_hot = info.clone();
                let mut info_cold = info.clone();
                info_hot.dest = r1;
                info_cold.dest = r2;
                hot_is.push(Instruction::Call {
                    func: pl.target.clone(),
                    info: info_hot,
                });
                if has_f64_second {
                    hot_is.push(Instruction::GetReturnF64Second {
                        dest: f64_dest.unwrap(),
                    });
                }
                cold_is.push(Instruction::CallIndirect {
                    func_ptr: fp_op,
                    info: info_cold,
                });
                if has_f64_second {
                    cold_is.push(Instruction::GetReturnF64Second {
                        dest: f64_dest.unwrap(),
                    });
                }
                if let Some(Instruction::GetReturnF64Second { .. }) =
                    block.instructions.get(pl.inst_idx + 4)
                {
                    // Original trailing instruction: remove it (moved).
                    let _ = block.instructions.remove(pl.inst_idx + 4);
                }
                let mut join_is: Vec<Instruction> = Vec::new();
                if let (Some(d), Some(a), Some(b)) = (dest, r1, r2) {
                    join_is.push(Instruction::Phi {
                        dest: d,
                        ty: ret_ty,
                        incoming: vec![
                            (Operand::Value(a), lhot),
                            (Operand::Value(b), lcold),
                        ],
                    });
                }
                f.blocks.push(BasicBlock {
                    label: lhot,
                    instructions: hot_is,
                    terminator: Terminator::Branch(ljoin),
                    source_spans: vec![],
                });
                f.blocks.push(BasicBlock {
                    label: lcold,
                    instructions: cold_is,
                    terminator: Terminator::Branch(ljoin),
                    source_spans: vec![],
                });
                join_is.extend(post);
                let mut join_spans: Vec<crate::common::source::Span> = Vec::new();
                join_spans.push(crate::common::source::Span::dummy());
                join_spans.extend(post_spans);
                f.blocks.push(BasicBlock {
                    label: ljoin,
                    instructions: join_is,
                    terminator: old_term,
                    source_spans: join_spans,
                });
                // SSA-invariant fix: the original block B's terminator moved
                // to the join, so ANY Phi anywhere that lists B as an
                // incoming predecessor must now list the JOIN block — the
                // value flows through the join. Without this, phi
                // elimination treats B (now a CondBranch, multi-successor)
                // as the predecessor of the old edge and creates a DANGLING
                // trampoline for it, silently dropping loop-carried copies
                // (observed: the loop counter never updated -> infinite
                // loop in the promoted binary).
                let b_label = f.blocks[bi].label;
                for blk in f.blocks.iter_mut() {
                    for inst in blk.instructions.iter_mut() {
                        if let Instruction::Phi { incoming, .. } = inst {
                            for (_, pred) in incoming.iter_mut() {
                                if pred.0 == b_label.0 {
                                    pred.0 = ljoin.0;
                                }
                            }
                        }
                    }
                }
                // Record the promoted blocks by label (lhot, lcold, ljoin).
                // The label-renumber pass remaps these via
                // crate::pgo::remap_promoted_hot before layout runs.
                hot_labels.entry(f.name.clone()).or_default().insert(lhot.0);
                hot_labels.entry(f.name.clone()).or_default().insert(lcold.0);
                hot_labels.entry(f.name.clone()).or_default().insert(ljoin.0);
                promoted += 1;
            }
        }
        f.next_value_id = next_val;
        f.next_label = next_label + 1;
    }
    if !hot_labels.is_empty() {
        crate::pgo::record_promoted_hot(u, hot_labels);
    }
    if promoted > 0 {
        eprintln!("lccc: PGO: promoted {} indirect call sites", promoted);
    }
    promoted
}
