//! Congruent recurrence-web merging (post-phi-elimination cleanup).
//!
//! Phi elimination lowers every loop-carried SSA phi to a COPY WEB: a set of
//! values joined by `Copy` edges, redefined once per back edge. When two phis
//! of the SAME loop had structurally identical definitions — same seed, same
//! increment — the webs they lower to compute the SAME VALUE at every program
//! point, yet nothing after phi elimination is allowed to merge them (GVN is
//! gone, and it could not see through the multi-def copy shape anyway).
//!
//! The shape this pass merges, exactly:
//!
//! ```text
//!     seed (block S):      Copy  D2 = X        // X identical for both webs
//!     increment (any dom): G2   = GEP(D2, K)   // or Add/Sub(D2, K)
//!     latch (block L):     Copy  D2 = G2       // G2's ONLY use
//! ```
//!
//! with W1's seed and latch in the SAME blocks S and L as W2's (so every
//! dynamic update of one fires with the other's). Under those conditions the
//! congruence `D1 ≡ D2` holds at every point by induction over the execution
//! trace: both start as X (the seed copies execute together in S), and every
//! latch in L steps both by the same constant K. Merging is therefore a pure
//! substitution: uses of D2 read D1, uses of G2 read G1.
//!
//! Why this matters for code quality: each un-merged web is an independent
//! register-allocation recurrence. linux_rbtree's insert loop carried THREE
//! webs for the single value `&node_pool[i]` (the field-access base, the
//! link-store value, and the key pointer — three source expressions whose
//! `node_pool + i*32` GEP chains were never CSE'd because two distinct
//! `GlobalAddr "node_pool"` seeds and two identical `Cast(i64)i` values
//! kept them textually distinct). The RA saw 3 slot round-trips per
//! iteration (9 instructions) where one `addq $32, %reg` (1 instruction)
//! computes the same thing — and the extra web pressure is what evicted the
//! loop's other state to slots in the first place.
//!
//! The pass runs AFTER `propagate_copies_post_phi` (which must see the
//! original copy shapes it was calibrated on) and BEFORE the edge-copy
//! layout (which wants the smallest possible web set). DCE removes the dead
//! seeds/increments/latch copies the rewrites strand.
//!
//! Kill switch: `CCC_NO_WEB_CONGRUENCE=1`.
//!
//! Soundness notes:
//! * The same-block requirement (S and L pairwise equal) is what makes the
//!   induction valid without any dominance reasoning: two copies in one
//!   block always execute together, so the webs can never diverge.
//! * `G`'s single-use requirement keeps the substitution trivially local:
//!   the only reader of the "next value" is the latch copy itself.
//! * Webs with more than the two canonical members, more than one seed or
//!   latch copy, non-constant increments, or increment opcodes other than
//!   GEP/Add/Sub are rejected (fail closed — they keep today's code).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{Instruction, IrBinOp, IrFunction, Operand, Value};

/// Union-find over value ids (copy-web builder).
struct WebSets {
    parent: FxHashMap<u32, u32>,
}

impl WebSets {
    fn new() -> Self {
        WebSets {
            parent: FxHashMap::default(),
        }
    }
    fn make(&mut self, v: u32) {
        self.parent.entry(v).or_insert(v);
    }
    fn find(&mut self, v: u32) -> u32 {
        self.make(v);
        let mut root = v;
        while let Some(&p) = self.parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        // Path compression.
        let mut cur = v;
        while let Some(&p) = self.parent.get(&cur) {
            if p == cur {
                break;
            }
            let next = p;
            self.parent.insert(cur, root);
            cur = next;
        }
        root
    }
    fn unite(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            // Deterministic root: smaller id wins.
            let (winner, loser) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent.insert(loser, winner);
        }
    }
}

/// The canonical recurrence shape a web must have to be mergeable. Two webs
/// with equal shapes are congruent (same seed operand, same seed/latch
/// blocks, same constant step).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct WebShape {
    /// The seed operand X, keyed by its debug representation (Operand is not
    /// Hash/Eq; the seed set is tiny so the string is cheap and total).
    seed: String,
    /// Block index of the seed copy.
    seed_block: usize,
    /// Block index of the latch copy.
    latch_block: usize,
    /// The increment structure.
    step: WebStep,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum WebStep {
    /// `G = GEP(D, K)`.
    Gep { offset: i64 },
    /// `G = D + K` (either operand order).
    Add { k: i64 },
    /// `G = D - K` (D on the lhs).
    Sub { k: i64 },
}

/// One analyzed candidate web.
struct Candidate {
    /// The multi-def copy member D.
    cursor: u32,
    /// The increment member G.
    step_value: u32,
    shape: WebShape,
}

pub(crate) fn merge_congruent_recurrence_webs(func: &mut IrFunction) -> usize {
    // ---- 1. Build recurrence webs over LATCH-shaped Copy edges. -------------
    // A Copy edge (D <- G) is a recurrence edge iff G's single def CONSUMES
    // D (`G = GEP(D, K)` / `Add(D, K)`): that is the phi-latch shape phi
    // elimination emits for a loop-carried value. Seed copies (`Copy D = X`,
    // X defined outside and independent of D) must NOT union anything: three
    // webs seeded from the same X are three separate recurrences until their
    // shapes prove them congruent, and unioning through X would fuse them
    // into one giant component before the shape analysis can see the
    // individual (seed, step) pairs.
    let mut reads_of: FxHashMap<u32, FxHashSet<u32>> = FxHashMap::default();
    {
        let mut read_events: Vec<(u32, u32)> = Vec::new();
        for block in &func.blocks {
            for inst in &block.instructions {
                let Some(dest) = inst.dest() else { continue };
                let dest = dest.0;
                crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        read_events.push((dest, v.0));
                    }
                });
                crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                    read_events.push((dest, v.0));
                });
            }
        }
        for (dest, read) in read_events {
            reads_of.entry(dest).or_default().insert(read);
        }
    }
    let mut webs = WebSets::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = inst
            {
                let src_reads_cursor = reads_of.get(&src.0).is_some_and(|rs| rs.contains(&dest.0));
                if src_reads_cursor {
                    webs.unite(dest.0, src.0);
                }
            }
        }
    }
    if webs.parent.is_empty() {
        return 0;
    }

    // ---- 2. Collect per-member def sites and use facts. ---------------------
    // defs of each value id: (block idx, instruction idx)
    let mut defs: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(dest) = inst.dest() {
                defs.entry(dest.0).or_default().push((bi, ii));
            }
        }
    }
    // Total uses per value, and the set of values with a use OUTSIDE a Copy
    // source position (instruction operand or terminator). A Copy source is
    // still a use — counted — but only non-copy-source uses disqualify a
    // step value from the single-latch-use contract.
    let mut use_events: Vec<(u32, bool)> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy { src, .. } = inst {
                if let Operand::Value(v) = src {
                    use_events.push((v.0, true));
                }
            } else {
                crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        use_events.push((v.0, false));
                    }
                });
                crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                    use_events.push((v.0, false));
                });
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                use_events.push((v.0, false));
            }
        });
    }
    let mut use_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut non_copy_use: FxHashSet<u32> = FxHashSet::default();
    for (vid, copy_src) in use_events {
        *use_count.entry(vid).or_insert(0) += 1;
        if !copy_src {
            non_copy_use.insert(vid);
        }
    }

    // ---- 3. Group members into webs and match the canonical shape. ----------
    let mut members_of: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    {
        let mut all: Vec<u32> = webs.parent.keys().copied().collect();
        all.sort_unstable();
        for &v in &all {
            let r = webs.find(v);
            members_of.entry(r).or_default().push(v);
        }
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (_root, members) in members_of.iter() {
        if members.len() != 2 {
            continue;
        }
        // Identify D (the member with 2+ all-Copy defs) and G (the member
        // with exactly one non-Copy def). The roles are unambiguous: a web
        // mixing any other def shape is rejected.
        let mut cursor: Option<u32> = None;
        let mut step_value: Option<u32> = None;
        for &m in members {
            let ds = defs.get(&m).map(|v| v.as_slice()).unwrap_or(&[]);
            let all_copies = ds.iter().all(|&(bi, ii)| {
                matches!(&func.blocks[bi].instructions[ii], Instruction::Copy { .. })
            });
            if !all_copies {
                if ds.len() == 1 && step_value.is_none() {
                    step_value = Some(m);
                } else {
                    step_value = None;
                    break;
                }
            } else if ds.len() >= 2 && cursor.is_none() {
                cursor = Some(m);
            }
        }
        let Some(cursor) = cursor else { continue };
        let Some(step_value) = step_value else {
            continue;
        };

        let cur_defs = match defs.get(&cursor) {
            Some(d) if d.len() == 2 => d.clone(),
            _ => continue, // exactly one seed + one latch
        };
        let step_defs = match defs.get(&step_value) {
            Some(d) if d.len() == 1 => d[0],
            _ => continue,
        };

        // Classify the two cursor copies into seed (src outside the web) and
        // latch (src == step_value).
        let member_set: FxHashSet<u32> = members.iter().copied().collect();
        let mut seed: Option<(Operand, usize)> = None;
        let mut latch: Option<usize> = None;
        for &(bi, ii) in &cur_defs {
            let Instruction::Copy { src, .. } = &func.blocks[bi].instructions[ii] else {
                continue;
            };
            match src {
                Operand::Value(v) if v.0 == step_value => {
                    if latch.is_some() {
                        seed = None;
                        break;
                    }
                    latch = Some(bi);
                }
                src => {
                    // Seed: source must be OUTSIDE the web (a const or a
                    // non-member value); a member source would be a third
                    // role this pass does not model.
                    if seed.is_some()
                        || matches!(src, Operand::Value(v) if member_set.contains(&v.0))
                    {
                        seed = None;
                        break;
                    }
                    seed = Some((src.clone(), bi));
                }
            }
        }
        let Some((seed_op, seed_block)) = seed else {
            continue;
        };
        let Some(latch_block) = latch else { continue };
        if seed_block == latch_block {
            continue;
        }
        let seed_key = format!("{:?}", seed_op);

        // The increment: G's single def, reading the cursor member, with a
        // constant step. G's only use must be the latch copy.
        if non_copy_use.contains(&step_value) || use_count.get(&step_value) != Some(&1) {
            continue;
        }
        let (sbi, sii) = step_defs;
        let step = match &func.blocks[sbi].instructions[sii] {
            Instruction::GetElementPtr { base, offset, .. } => {
                if base.0 != cursor {
                    continue;
                }
                let Operand::Const(k) = offset else { continue };
                let Some(k) = k.to_i64() else { continue };
                WebStep::Gep { offset: k }
            }
            Instruction::BinOp { op, lhs, rhs, .. } => {
                let (member_lhs, other) = match (lhs, rhs) {
                    (Operand::Value(l), r) if l.0 == cursor => (true, r),
                    (l, Operand::Value(r)) if r.0 == cursor => (false, l),
                    _ => continue,
                };
                let Operand::Const(k) = other else { continue };
                let Some(k) = k.to_i64() else { continue };
                match op {
                    IrBinOp::Add => WebStep::Add { k },
                    IrBinOp::Sub if member_lhs => WebStep::Sub { k },
                    _ => continue,
                }
            }
            _ => continue,
        };

        candidates.push(Candidate {
            cursor,
            step_value,
            shape: WebShape {
                seed: seed_key,
                seed_block,
                latch_block,
                step,
            },
        });
    }
    if candidates.len() < 2 {
        return 0;
    }

    // ---- 4. Group by shape; merge every later web into the first. -----------
    let mut by_shape: FxHashMap<WebShape, Vec<usize>> = FxHashMap::default();
    for (i, c) in candidates.iter().enumerate() {
        by_shape.entry(c.shape.clone()).or_default().push(i);
    }
    let mut subst: FxHashMap<u32, u32> = FxHashMap::default();
    let mut merged = 0usize;
    for (_shape, group) in by_shape {
        if group.len() < 2 {
            continue;
        }
        let primary = &candidates[group[0]];
        for &ci in &group[1..] {
            let secondary = &candidates[ci];
            if secondary.cursor == primary.cursor {
                continue;
            }
            subst.insert(secondary.cursor, primary.cursor);
            subst.insert(secondary.step_value, primary.step_value);
            merged += 1;
        }
    }
    if merged == 0 {
        return 0;
    }

    // ---- 5. Substitute uses everywhere (instructions + terminators). --------
    let rewrite_val = |v: &mut Value| {
        if let Some(&to) = subst.get(&v.0) {
            *v = Value(to);
        }
    };
    let rewrite_op = |op: &mut Operand| {
        if let Operand::Value(v) = op {
            rewrite_val(v);
        }
    };
    for block in func.blocks.iter_mut() {
        for inst in &mut block.instructions {
            inst.for_each_operand_mut(rewrite_op);
            inst.for_each_value_use_mut(rewrite_val);
        }
        block.terminator.for_each_operand_mut(rewrite_op);
    }
    merged
}
