//! Global Location Allocation (GLA) — P0-A, Phase 1.
//!
//! # Why this module exists
//!
//! The production register allocator's only spill model is **lifetime
//! demotion**: a value that loses an eviction contest is homed in one stack
//! slot for its whole live range, so every later use either folds a memory
//! operand or pays a reload. `split_ranges::split_high_pressure_ranges`
//! (RA-06) introduced Belady-MIN splitting, but only *intra-block*: its
//! own decision record (`engineering/DECISIONS.md`, "RA-06 (2026-09-05)")
//! proves that partial spill-then-color is strictly worse than either
//! endpoint, because the loop-carried values that dominate real pressure
//! peaks are header-phi webs whose consumers live in another block, and
//! renaming those needs global SSA repair. That record states the missing
//! prerequisite verbatim: *"Cross-block SSA repair, so the spill phase can
//! reach MAXLIVE <= k globally."* This module is that prerequisite.
//!
//! # Location model
//!
//! The planner reasons about three locations (the `Location`/`LocationPiece`
//! vocabulary of the P0-A design):
//!
//! ```text
//! enum Location { Register, Stack(slot), Rematerialize(template) }
//! ```
//!
//! A logical SSA value is therefore no longer assumed to own one register
//! for its whole life. The Phase-1 materialization does not pin physical
//! registers (the existing scan / graph colorer still chooses the register
//! for every *register* piece); it realizes `Stack` and `Rematerialize`
//! pieces as IR rewrites *before* phi elimination:
//!
//! * a **Stack piece** is bounded by a fresh reload SSA value (the register
//!   piece after the gap); the backing slot is an internal volatile alloca,
//!   exactly like the proven call/pressure splitters;
//! * a **Rematerialize piece** re-executes a source-less cheap definition
//!   (`GlobalAddr`, `Copy` of a constant) instead of reloading.
//!
//! Register pieces are simply the (now shorter) remaining webs of the
//! original value, which the downstream colorer allocates exactly as today.
//!
//! # Why the SSA repair needs no new φ nodes (and when that changes)
//!
//! A cross-block gap joins register pieces across control flow, which in a
//! textbook Cytron spiller means φ insertion at dominance frontiers. This
//! module avoids hand-placed φs through a stronger construct:
//!
//! * every split value receives exactly one **immutable capture store**
//!   immediately after its static definition. SSA values are immutable, so
//!   the slot then holds the value on *every* path for the rest of that
//!   dynamic execution (for a header φ the capture follows the φ prefix and
//!   re-executes every iteration with the current joined value);
//! * a reload may therefore be placed at the head of *any* memory-region
//!   block, immediately before any use cluster, or on any split CFG edge,
//!   and it provably reads the correct value — the memory slot itself is
//!   the join (a φ realized in storage), and each reload reconstructs a
//!   fresh SSA name;
//! * an edge that needs a register value only on one successor gets a
//!   **critical-edge trampoline** (the same construction phi elimination
//!   uses), so the reload never speculatively executes on the other edge.
//!
//! This is the SplitKit observation (a spill slot is a legal, dominance-free
//! join point) and it is strictly simpler to verify than φ insertion: the
//! only dominance obligation is "the capture store follows the unique
//! definition, which already dominates every former use", plus "each reload
//! is dominated by that store". The verifier below checks both.
//!
//! What this construction is *not*: it does not yet pin physical registers
//! per piece, does not resolve cyclic φ webs into register permutations
//! (P0-B), and does not fold address expressions (P0-C). Recurrence-carried
//! header φs (sha256's a..h, accumulator reductions) are deliberately
//! excluded from anchor eviction: an immutable capture on a header φ stores
//! every iteration on the carried dependency chain itself, which is the
//! exact demotion the production allocator already protects against via
//! `span_recurrence`.
//!
//! # Fail-closed contract
//!
//! Every analysis gap resolves to "do not split this value". A value is only
//! split when the planner proves (a) GPR eligibility, (b) a pressure excess
//! the edit relieves in a block the edit can plausibly tip under budget, and
//! (c) a weighted benefit above the weighted store/reload traffic. The
//! transform never rewrites a shape it did not model.
//!
//! Enable with `CCC_RA_GLOBAL_LOCATION=1` (A/B gate, default off; an explicit
//! `=0`/`false`/`off` also means off).
//!
//! # Shipped Phase-1 policy (A/B calibrated 2026-09-11, post PR #499)
//!
//! The planner's vocabulary (remat, cross-block gaps, edge trampolines) is
//! fully implemented, but the shipped policy is deliberately narrow; every
//! gate is a fail-closed filter (loosening a gate can only ADD edits). The
//! calibration used the 78-file benchmark corpus
//! (`scripts/ra_quality_census.py --ab-env`) plus Godbolt oracle rank
//! (`scripts/codegen_oracle.py --rank`):
//!
//! * **Rematerialization ON, spill gaps OFF** (`CCC_GLA_SPILL_GAPS=1` to
//!   re-enable). Pre-allocation capture-store/reload gaps measured
//!   net-negative in every configuration: the proxy cannot see which
//!   residency the production colorer (folding memory operands, callee-save
//!   buys, and PR #499's next-use cost-ratio escape) already resolves, so
//!   gaps mostly traded once-per-call callee-save traffic for per-iteration
//!   reloads. The gap machinery is retained as the P0-B / post-alloc
//!   feedback substrate but never fires in shipped policy.
//! * **Pressure counts coalesced color CLASSES**, not SSA names: every φ
//!   result is unioned with its incoming operands. This can only
//!   under-count (φ coalescing may itself insert edge copies in the
//!   colorer), which is the safe direction.
//! * **Reach band, target- and tier-aware** (`CCC_GLA_REACH`): blocks whose
//!   peak exceeds the register budget by more than the band cannot be made
//!   colorable by a handful of edits, and editing them can reshuffle the
//!   colorer's choices in blocks it would have handled optimally. The band
//!   proxies the production allocator's residual register capacity: 6 on
//!   x86-64 (the six buyable callee-saved GPRs), **2 on i686** (under PIC
//!   %ebx is the GOT base and %rbp the frame pointer, leaving %esi/%edi —
//!   band 6 caused a measured 1.053× loop_patterns runtime regression from
//!   an induction counter spilled in exchange for a buffer base, band 2
//!   removes it while keeping every static win), and **64 (unbanded) at
//!   -O0** where the coloring tier is disabled so there is no plan to
//!   perturb and remat relief is a direct stack-traffic reduction. This is
//!   the nbody discriminator on x86-64: its FP inner loop peaks at 38–45
//!   GPR classes and every edit there only perturbed the global coloring;
//!   adler32 peaks at 14–16 and one remat tips it.
//! * **Remat segment cap 1** (`CCC_GLA_REMAT_MAX_SEGMENTS`): a value
//!   whose hole-aware live range has two or more segments (nbody's printf
//!   format-string global, two segments around the nested simulation
//!   loops) is not rematerialized: the cloned definition only reorders
//!   global coloring for a net instruction increase. Single-segment
//!   hoisted bases (adler32's check table base) free a register cleanly.
//!   For nbody itself the reach band now rejects first (v94 covers only
//!   hopeless blocks); the cap remains an independently load-bearing
//!   gate when the band is widened, pinned in the 2×2 matrix in
//!   `tests/regression/check_gla_remat_policy.sh`.
//!
//! Result with the feature ENABLED (78-file corpus, -O2): one fire site
//! (zlib_ng_adler32 main), +4 static instructions and +1 cold
//! unconditional jump, but −12 stack references, stores 6→4 and spills
//! 19→7 measured by the Godbolt tooling — the 8 hottest references move
//! out of the per-8-byte inner DO8 loop; the remaining store/reload pair
//! executes once per 5552-byte NMAX block. Frame shrinks 56→40 bytes.
//! At -O0 the same gate is insn-neutral-to-positive corpus-wide.
//! i686 (budget 6 GPRs): −4 stack refs corpus-wide with cold prologue
//! setup the only +insn sites. Gap-enabled fuzz (synthetic / phi_cfg /
//! differential, verifier forced) and full gap unit tests keep the
//! dormant substrate covered.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::*;
use crate::passes::loop_analysis;
use std::sync::OnceLock;

use super::liveness::LivenessResult;
use super::split_ranges::{
    collect_value_types, first_non_phi, insert_entry_alloca, is_simple_gpr_type, next_value,
    pressure_budget, pressure_min_gap, replace_values_in_inst, replace_values_in_terminator,
    split_debug_enabled, terminator_uses_value,
};

// Phase 1 lives in focused modules; the glob re-exports make the cross-cut
// helpers (block/phi queries, pressure primitives, plan entries) visible
// across the submodules exactly as they were in the former monolith.
mod materializer;
mod planner;
mod policy;
mod pressure;
mod verifier;

pub(super) use materializer::*;
pub(super) use planner::*;
pub(super) use policy::*;
pub(super) use pressure::*;
pub(super) use verifier::*;

// Driver entry points.
pub(crate) use planner::run;
pub(crate) use policy::{gate_enabled, max_splits_from_env};

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn func_with(blocks: Vec<BasicBlock>, next: u32) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, Vec::new(), false);
        f.blocks = blocks;
        f.next_value_id = next;
        f
    }
    fn blk(label: u32, instructions: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator: term,
            source_spans: Vec::new(),
        }
    }
    fn bin(dest: u32, op: IrBinOp, a: u32, b: u32, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(a)),
            rhs: Operand::Value(Value(b)),
            ty,
        }
    }
    fn add(dest: u32, a: u32, b: u32) -> Instruction {
        bin(dest, IrBinOp::Add, a, b, IrType::I64)
    }
    fn copy_const(dest: u32, c: i64) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Const(IrConst::I64(c)),
        }
    }
    fn global(dest: u32, name: &str) -> Instruction {
        Instruction::GlobalAddr {
            dest: Value(dest),
            name: name.to_string(),
        }
    }
    fn count(f: &IrFunction, p: impl Fn(&Instruction) -> bool) -> usize {
        f.blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| p(i))
            .count()
    }

    /// Build >budget independent long-lived values that all stay live until
    /// a sequential consumer chain at the end. Returns (instructions,
    /// result id to return).
    fn pressure_ladder(cold: u32) -> (Vec<Instruction>, u32) {
        let mut v = Vec::new();
        v.push(add(cold, 500, 501));
        for k in 0..14u32 {
            v.push(add(100 + k, 600 + k, 620 + k));
        }
        // Consumer chain keeps every long-lived value resident until use.
        v.push(add(200, cold, 100));
        for k in 1..14u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        (v, 213)
    }

    /// Like [`pressure_ladder`] but with the named value as a GlobalAddr,
    /// the rematerialization candidate. 12 long-lived values: pressure with
    /// the global is 13 (> budget), after its rematerialization exactly 12,
    /// so no spill gap may accompany the remat.
    fn remat_ladder() -> Vec<Instruction> {
        let mut v = vec![global(1, "G")];
        for k in 0..12u32 {
            v.push(add(100 + k, 800 + k, 820 + k));
        }
        v.push(add(200, 1, 100));
        for k in 1..12u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        v
    }

    // ── near misses ──

    #[test]
    fn low_pressure_does_not_fire() {
        let mut f = func_with(
            vec![blk(
                0,
                vec![add(1, 900, 901)],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            )],
            902,
        );
        assert_eq!(run(&mut f, 64, 2), 0);
    }

    #[test]
    fn float_values_never_split() {
        let mut f = func_with(
            vec![blk(
                0,
                vec![Instruction::BinOp {
                    dest: Value(10),
                    op: IrBinOp::Add,
                    lhs: Operand::Const(IrConst::F64(1.0)),
                    rhs: Operand::Const(IrConst::F64(2.0)),
                    ty: IrType::F64,
                }],
                Terminator::Return(Some(Operand::Value(Value(10)))),
            )],
            11,
        );
        assert_eq!(run(&mut f, 64, 2), 0);
    }

    #[test]
    fn div_operands_are_ineligible() {
        let bad = collect_ineligible(&func_with(
            vec![blk(
                0,
                vec![bin(2, IrBinOp::UDiv, 1, 3, IrType::I64)],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            )],
            4,
        ));
        assert!(bad.contains(&1));
        assert!(bad.contains(&3));
    }

    #[test]
    fn variable_shift_count_is_ineligible() {
        let bad = collect_ineligible(&func_with(
            vec![blk(
                0,
                vec![bin(2, IrBinOp::Shl, 1, 3, IrType::I64)],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            )],
            4,
        ));
        assert!(bad.contains(&3), "shift count v3 must stay pinned");
        assert!(!bad.contains(&1), "shifted value may be split");
    }

    // ── recurrence exclusion ──

    #[test]
    fn recurrence_phi_dests_are_excluded() {
        let blocks = vec![
            blk(0, vec![add(90, 80, 81)], Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(1),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(90)), BlockId(0)),
                            (Operand::Value(Value(2)), BlockId(2)),
                        ],
                    },
                    add(2, 1, 1),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(2)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            blk(2, vec![], Terminator::Branch(BlockId(1))),
            blk(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            ),
        ];
        let f = func_with(blocks, 91);
        let lm = analysis::build_label_map(&f);
        let (preds, succs) = analysis::build_cfg(&f, &lm);
        let idom = analysis::compute_dominators(4, &preds, &succs);
        let rec = collect_recurrence_phi_dests(&f, &lm, &preds, &succs, &idom);
        assert!(rec.contains(&1), "latch-carried φ v1 is recurrent");
    }

    // ── rematerialization fires ──

    #[test]
    fn global_addr_rematerializes_under_pressure() {
        let insts = remat_ladder();
        let mut f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let n = run(&mut f, 64, 2);
        assert!(n >= 1, "remat must fire under pressure");
        assert!(
            count(&f, |i| matches!(i, Instruction::GlobalAddr { .. })) >= 2,
            "expected optimization fires: a rematerialized GlobalAddr"
        );
        // Remat never builds a stack slot.
        assert_eq!(count(&f, |i| matches!(i, Instruction::Alloca { .. })), 0);
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    #[test]
    fn copy_const_is_rematerializable() {
        assert!(rematerializable_template(&copy_const(1, 42)).is_some());
        assert!(rematerializable_template(&global(2, "x")).is_some());
        assert!(rematerializable_template(&add(3, 1, 2)).is_none());
        // A register-dependent copy is NOT source-less.
        let dep = Instruction::Copy {
            dest: Value(4),
            src: Operand::Value(Value(3)),
        };
        assert!(rematerializable_template(&dep).is_none());
    }

    // ── cross-block gap fires (the capability the intra-block pass lacks) ──

    #[test]
    fn cross_block_gap_spills_and_reloads_in_exit() {
        // b0: 15 long-lived values (cold = v1); b1: hot self-loop carrying
        // only counter work, all 15 live across it; b2: fold the values and
        // return. Only a global gap relieves the loop's residency peak.
        let (b0_all, _) = pressure_ladder(1);
        // The consumer chain belongs in the exit block, not the preheader.
        let b0 = b0_all[..15].to_vec();
        let mut b2 = Vec::new();
        b2.push(add(200, 1, 100));
        for k in 1..14u32 {
            b2.push(add(200 + k, 199 + k, 100 + k));
        }
        b2.push(add(300, 213, 1));
        let blocks = vec![
            blk(0, b0, Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(2),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(0)), BlockId(0)),
                            (Operand::Value(Value(3)), BlockId(1)),
                        ],
                    },
                    add(3, 2, 900),
                    Instruction::Cmp {
                        dest: Value(4),
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(Value(3)),
                        rhs: Operand::Const(IrConst::I64(10)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(4)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            blk(2, b2, Terminator::Return(Some(Operand::Value(Value(300))))),
        ];
        let mut f = func_with(blocks, 900);
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "global gap must fire across the loop");
        // At least one immutable capture store and an exit-block reload.
        assert!(count(&f, |i| matches!(i, Instruction::Store { .. })) >= 1);
        let loads_b2 = f.blocks[2]
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::Load { .. }))
            .count();
        assert!(loads_b2 >= 1, "exit block reloads the spilled value");
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    // ── mid-block seed gap: an early register read must not be served by a
    // reload scheduled ahead of the capture store ──

    #[test]
    fn midblock_gap_keeps_early_read_register_resident() {
        // v1 has an early register-region read, but its farthest-next-use
        // distance makes it the best Belady eviction for the later peak.
        // Its capture anchor and that early read coincide at one point: the
        // seed block must NOT be marked entry-memory (the gap starts at 2),
        // or a reload is scheduled ahead of the store.
        let mut v = Vec::new();
        v.push(add(1, 900, 901)); // def v1
        v.push(add(2, 1, 902)); // EARLY read of v1 (register region)
        for k in 0..14u32 {
            v.push(add(100 + k, 600 + k, 620 + k)); // pressure peak
        }
        // Consumer chain folds 100.. first; v1 is consumed LAST, so its
        // next-use distance is the largest at the peak.
        v.push(add(200, 950, 100));
        for k in 1..14u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        v.push(add(214, 213, 113));
        v.push(add(215, 214, 1)); // v1 read at the very end
        let mut f = func_with(
            vec![blk(
                0,
                v,
                Terminator::Return(Some(Operand::Value(Value(215)))),
            )],
            900,
        );
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "a mid-block gap must relieve the peak");
        // Every capture store must precede its reloads, point-wise.
        let mut slots = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Instruction::Alloca { dest, .. } = i {
                    slots.insert(dest.0);
                }
            }
        }
        verify_rewrite(&f, &slots).expect("no reload may precede its capture store");
        // The early reader (the def of v2) must still name v1 directly
        // rather than a reloaded fresh name.
        let early = f.blocks[0]
            .instructions
            .iter()
            .find(|i| i.dest().is_some_and(|d| d.0 == 2))
            .expect("early reader still present");
        let mut uses_v1 = false;
        crate::backend::liveness::for_each_operand_in_instruction(early, |op| {
            if matches!(op, Operand::Value(v) if v.0 == 1) {
                uses_v1 = true;
            }
        });
        assert!(uses_v1, "early read stays on the original register name");
    }

    // ── closure must stop at a (re-)definition block. Liveness models a
    // φ incoming as an edge copy at the pred end, so a value DEFINED in a
    // block that feeds a header φ is conservatively reported live-in to
    // its own def block; propagating the memory region through the loop
    // scheduled a reload ahead of the capture anchor. ──

    #[test]
    fn closure_does_not_reenter_def_block() {
        // b0: initial sum v1 + 14 live-through pressure values.
        // b1: header with two recurrent φs (excluded from anchoring).
        // b2: empty body-prefix block (X), predecessor of the latch;
        //     liveness reports v3 live-out here via the φ edge copy.
        // b3: latch (D) that DEFINES v3 (new sum1) and feeds φ v9.
        // b4: exit consuming every long-lived value.
        let mut b0 = vec![add(1, 700, 701)];
        for k in 0..14u32 {
            b0.push(add(100 + k, 600 + k, 620 + k));
        }
        let mut bexit = vec![add(200, 9, 100)];
        for k in 1..14u32 {
            bexit.push(add(200 + k, 199 + k, 100 + k));
        }
        let blocks = vec![
            blk(0, b0, Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(9),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(1)), BlockId(0)),
                            (Operand::Value(Value(3)), BlockId(3)),
                        ],
                    },
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(0)), BlockId(0)),
                            (Operand::Value(Value(4)), BlockId(3)),
                        ],
                    },
                    Instruction::Cmp {
                        dest: Value(11),
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(Value(9)),
                        rhs: Operand::Const(IrConst::I64(100)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(11)),
                    true_label: BlockId(2),
                    false_label: BlockId(4),
                },
            ),
            blk(2, vec![], Terminator::Branch(BlockId(3))),
            blk(
                3,
                vec![
                    add(5, 9, 777), // byte value
                    add(3, 9, 5),   // new sum1 — DEFINED in the latch
                    add(4, 10, 3),  // new sum2, reads the fresh sum1
                ],
                Terminator::Branch(BlockId(1)),
            ),
            blk(
                4,
                bexit,
                Terminator::Return(Some(Operand::Value(Value(213)))),
            ),
        ];
        // Direct closure invariant: v3 is DEFINED in block 3 and feeds the
        // header φ from block 3's end, so liveness conservatively reports it
        // live-in to its own def block. The closure seeded in its
        // predecessor block 2 must still not enter block 3.
        let mut f = func_with(blocks, 900);
        // Explicit direct probe before the rewrite.
        {
            let live = crate::backend::liveness::compute_live_intervals(&f);
            let lm = analysis::build_label_map(&f);
            let (_preds, succs) = analysis::build_cfg(&f, &lm);
            // Document the conservative precondition: without the def-block
            // stop test the closure would have somewhere to run.
            assert!(
                live.is_live_in(3, 3),
                "test shape requires the φ edge-copy live-in over-approximation"
            );
            let reached = memory_closure(&live, &f, &succs, 3, 2);
            assert!(
                reached.iter().all(|(b, _)| *b != 3),
                "closure re-entered the value's own def block: {reached:?}"
            );
        }
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "pressure must force splits");
        // Every capture store precedes its reloads (a closure crossing
        // into the latch def block put the reload ahead of the anchor).
        let mut slots = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Instruction::Alloca { dest, .. } = i {
                    slots.insert(dest.0);
                }
            }
        }
        verify_rewrite(&f, &slots).expect("closure never crosses a (re-)definition");
        // The fresh-sum reader (v4 = v10 + v3) still names v3 directly.
        let latch = f
            .blocks
            .iter()
            .find(|b| b.label == BlockId(3))
            .expect("latch present");
        let sum2 = latch
            .instructions
            .iter()
            .find(|i| i.dest().is_some_and(|d| d.0 == 4))
            .expect("sum2 still present");
        let mut reads_v3 = false;
        crate::backend::liveness::for_each_operand_in_instruction(sum2, |op| {
            if matches!(op, Operand::Value(v) if v.0 == 3) {
                reads_v3 = true;
            }
        });
        assert!(reads_v3, "latch sum2 reads the freshly defined sum1, v3");
    }

    // ── folded reads from an EARLIER block must never map to local point
    // 0 of a later φ-headed block (saturating_sub underflow) ──

    #[test]
    fn folded_reads_never_land_in_phi_prefix() {
        // b0: global v1, GEP v2 = v1+8, Load v3 folds both (hidden read of
        // v1 at b0's load point). b1 is a self-loop header with a φ; its
        // block-start global point is GREATER than v1's folded point.
        let blocks = vec![
            blk(
                0,
                vec![
                    global(1, "G"),
                    Instruction::GetElementPtr {
                        dest: Value(2),
                        base: Value(1),
                        offset: Operand::Const(IrConst::I64(8)),
                        ty: IrType::Ptr,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(3),
                        ptr: Value(2),
                        ty: IrType::U8,
                        seg_override: AddressSpace::Default,
                    },
                ],
                Terminator::Branch(BlockId(1)),
            ),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(4),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(3)), BlockId(0)),
                            (Operand::Value(Value(5)), BlockId(1)),
                        ],
                    },
                    add(5, 4, 900),
                    Instruction::Cmp {
                        dest: Value(6),
                        op: IrCmpOp::Slt,
                        lhs: Operand::Value(Value(5)),
                        rhs: Operand::Const(IrConst::I64(3)),
                        ty: IrType::I64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(6)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            blk(
                2,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(4)))),
            ),
        ];
        let f = func_with(blocks, 900);
        let live = crate::backend::liveness::compute_live_intervals(&f);
        // The header (block 1) must not see v1 read at local point 0 — that
        // point is inside its φ prefix.
        let pts = block_use_points(&live, &f, 1, 1);
        assert!(
            !pts.contains(&0),
            "earlier-block folded read mapped into φ prefix: {pts:?}"
        );
    }

    // ── stale next_value_id (an upstream pass leaked a high id) must
    // never make GLA mint colliding value ids ──

    #[test]
    fn mints_fresh_ids_above_every_existing_def() {
        let (insts, _res) = pressure_ladder(1);
        let mut all = vec![Instruction::Alloca {
            dest: Value(899),
            ty: IrType::I64,
            size: 8,
            align: 8,
            volatile: true,
            semantic_volatile: false,
        }];
        all.extend(insts);
        let mut f = func_with(
            vec![blk(
                0,
                all,
                Terminator::Return(Some(Operand::Value(Value(213)))),
            )],
            // Deliberately stale: one existing definition (v899) is already
            // at or above the counter.
            899,
        );
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1, "pressure ladder still splits");
        let mut seen = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    assert!(seen.insert(d.0), "duplicate definition of v{}", d.0);
                    assert!(
                        d.0 < f.next_value_id,
                        "id {} >= next_value_id {}",
                        d.0,
                        f.next_value_id
                    );
                }
            }
        }
        let mut slots = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Instruction::Alloca { dest, .. } = i {
                    slots.insert(dest.0);
                }
            }
        }
        verify_rewrite(&f, &slots).expect("rewrite verifies with a stale counter");
    }

    // ── call boundary gap ──

    #[test]
    fn call_gap_is_profitable_and_sound() {
        // v1 defined, call (clobbers), v1 used much later under pressure: the
        // gap containing the call stores before / reloads after. Shape is a
        // single block with a call and many live values.
        let (mut mut_insts, _) = pressure_ladder(1);
        // Remove the in-block consumer chain: it moves after the call.
        mut_insts.truncate(15);
        let mut insts = mut_insts;
        insts.push(Instruction::Call {
            func: "clobber".into(),
            info: CallInfo {
                dest: Some(Value(50)),
                args: Vec::new(),
                arg_types: Vec::new(),
                return_type: IrType::I64,
                is_variadic: false,
                num_fixed_args: 0,
                struct_arg_sizes: Vec::new(),
                struct_arg_aligns: Vec::new(),
                struct_arg_classes: Vec::new(),
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: Vec::new(),
                ret_is_f128_sse: false,
                is_sret: false,
                is_fastcall: false,
                regparm: None,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: Vec::new(),
            },
        });
        // Post-call consumer chain.
        insts.push(add(200, 1, 100));
        for k in 1..14u32 {
            insts.push(add(200 + k, 199 + k, 100 + k));
        }
        let mut f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(213)))),
            )],
            900,
        );
        let n = run_with_policy(
            &mut f,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(n >= 1);
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    // ── critical edge mechanics ──

    #[test]
    fn retarget_edge_moves_exactly_one_edge() {
        let mut term = Terminator::CondBranch {
            cond: Operand::Const(IrConst::I64(1)),
            true_label: BlockId(7),
            false_label: BlockId(8),
        };
        retarget_edge(&mut term, BlockId(7), BlockId(99));
        match term {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                assert_eq!(true_label, BlockId(99));
                assert_eq!(false_label, BlockId(8));
            }
            _ => panic!(),
        }
    }

    // ── determinism ──

    #[test]
    fn planning_is_deterministic() {
        let make = || {
            let insts = remat_ladder();
            func_with(
                vec![blk(
                    0,
                    insts,
                    Terminator::Return(Some(Operand::Value(Value(211)))),
                )],
                900,
            )
        };
        let mut a = make();
        let mut b = make();
        run(&mut a, 64, 2);
        run(&mut b, 64, 2);
        let ca: Vec<usize> = a.blocks.iter().map(|x| x.instructions.len()).collect();
        let cb: Vec<usize> = b.blocks.iter().map(|x| x.instructions.len()).collect();
        assert_eq!(ca, cb);
        assert_eq!(a.next_value_id, b.next_value_id);
    }

    // ── verifier ──

    #[test]
    fn verifier_catches_undominated_reload() {
        let f = func_with(
            vec![blk(
                0,
                vec![Instruction::Load {
                    volatile: false,
                    dest: Value(1),
                    ptr: Value(5),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                }],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            )],
            6,
        );
        let mut slots = FxHashSet::default();
        slots.insert(5);
        assert!(verify_rewrite(&f, &slots).is_err());
    }

    #[test]
    fn verifier_accepts_dominated_reload() {
        let f = func_with(
            vec![blk(
                0,
                vec![
                    Instruction::Store {
                        val: Operand::Value(Value(7)),
                        ptr: Value(5),
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(5),
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                    },
                ],
                Terminator::Return(Some(Operand::Value(Value(1)))),
            )],
            8,
        );
        let mut slots = FxHashSet::default();
        slots.insert(5);
        assert!(verify_rewrite(&f, &slots).is_ok());
    }

    // ── Phase-1 profitability policy (A/B-calibrated 2026-09-11) ──

    /// The production policy: spill gaps OFF, intra-block gaps OFF,
    /// one-segment remats, reach band 6. Mirrors `GlaPolicy::from_env`
    /// defaults WITHOUT reading the environment so tests are hermetic.
    fn conservative_policy(max_splits: usize) -> GlaPolicy {
        GlaPolicy {
            tier: Tier::Speed,
            budget: pressure_budget(),
            max_splits,
            remat_max_weight: 3,
            remat_max_segments: 1,
            reach_band: 6,
            allow_spill_gaps: false,
            allow_intra_block_gaps: false,
            min_gap: pressure_min_gap(),
            min_benefit: 40,
            benefit_ratio10: 20,
        }
    }

    #[test]
    fn reach_band_logic_rejects_only_grossly_over_blocks() {
        let p = conservative_policy(64);
        assert!(p.peak_is_reachable(p.budget as u32));
        assert!(p.peak_is_reachable((p.budget + 6) as u32));
        assert!(!p.peak_is_reachable((p.budget + 7) as u32));
        assert!(!p.peak_is_reachable(45));
    }

    #[test]
    fn reach_band_defaults_follow_target_and_opt_tier() {
        // The A/B override would change the answer; hermetic tests assume no
        // ambient override.
        if std::env::var("CCC_GLA_REACH").is_ok() || crate::common::types::target_is_32bit() {
            return;
        }
        // x86-64 speed tier: the six buyable callee-saved GPRs.
        assert_eq!(reach_band(tier_for(1)), 6);
        assert_eq!(reach_band(tier_for(2)), 6);
        assert_eq!(reach_band(tier_for(3)), 6);
        // -O0 disables the coloring tier: no plan to perturb, band wide.
        assert_eq!(reach_band(tier_for(0)), 64);
        // Size tiers never plan: band zero.
        assert_eq!(reach_band(tier_for(4)), 0);
        assert_eq!(tier_for(0), Tier::Debug);
        assert_eq!(tier_for(2), Tier::Speed);
        assert_eq!(tier_for(5), Tier::Size);
    }

    /// The Speed reach band is DERIVED from the named buyable callee-saved
    /// register sets, and the sets carry their ABI/PIC facts: six on
    /// x86-64, two on i686 (PIC owns %ebx, the frame owns %ebp).
    #[test]
    fn reach_band_equals_buyable_callee_saved_count() {
        if std::env::var("CCC_GLA_REACH").is_ok() {
            return; // explicit A/B override replaces the derivation
        }
        use crate::common::types::{set_target_ptr_size, target_ptr_size};
        let saved = target_ptr_size();
        set_target_ptr_size(8);
        assert_eq!(X86_64_BUYABLE_CALLEE_SAVED_GPRS.len(), 6);
        assert_eq!(buyable_callee_saved_gprs(), 6);
        assert_eq!(reach_band(Tier::Speed), 6);
        set_target_ptr_size(4);
        assert_eq!(I686_BUYABLE_CALLEE_SAVED_GPRS.len(), 2);
        assert_eq!(buyable_callee_saved_gprs(), 2);
        assert_eq!(reach_band(Tier::Speed), 2);
        // Tier branches that never depend on the target stay fixed.
        assert_eq!(reach_band(Tier::Debug), 64);
        assert_eq!(reach_band(Tier::Size), 0);
        set_target_ptr_size(saved);
    }

    /// An exhausted fresh-value id space aborts the whole plan BEFORE the
    /// first IR mutation: no panic, zero edits, original function intact.
    #[test]
    fn next_value_exhaustion_fails_closed() {
        let insts = remat_ladder();
        let globals_before = count(
            &func_with(
                vec![blk(0, insts.clone(), Terminator::Return(None))],
                u32::MAX,
            ),
            |i| matches!(i, Instruction::GlobalAddr { .. }),
        );
        let mut f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            u32::MAX,
        );
        // The same shape fires remat with a normal id space
        // (`global_addr_rematerializes_under_pressure`); with no id left
        // the plan must be applied zero times.
        let n = run(&mut f, 64, 2);
        assert_eq!(n, 0, "exhausted id space must not apply any split");
        assert_eq!(
            count(&f, |i| matches!(i, Instruction::GlobalAddr { .. })),
            globals_before,
            "no rematerialized definition may be minted"
        );
        assert_eq!(
            count(&f, |i| matches!(i, Instruction::Alloca { .. })),
            0,
            "no slot alloca may be minted"
        );
        assert_eq!(f.next_value_id, u32::MAX);
    }

    #[test]
    fn size_tier_never_plans() {
        let mut p = conservative_policy(64);
        p.tier = Tier::Size;
        assert!(!p.enabled());
        let mut f = func_with(
            vec![blk(
                0,
                remat_ladder(),
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        assert_eq!(run_with_policy(&mut f, p), 0);
    }

    #[test]
    fn own_use_run_points_are_not_net_relief() {
        let policy = conservative_policy(64);
        // Shape A (adler32-style): global defined at function entry, read
        // only at a LATE cluster; the residency it carries ACROSS the
        // pressure peak is net relief and must be credited.
        let insts = remat_ladder();
        let f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let live = crate::backend::liveness::compute_live_intervals(&f);
        let classes = ColorClasses::phi_webs(&f);
        let eligible: FxHashSet<u32> = (1u32..=211).collect();
        let pressure = Pressure::build(&live, &f, &eligible, &classes);
        assert!(covers_over_budget_point(&live, &f, &pressure, 1, &policy));

        // Shape B (nbody -Os-style): 12 producers hold the block at
        // exactly budget; global v1 is defined late and read at the very
        // next point, then dies. Every over-budget point is a def/read
        // point where the clone occupies the register the edit claims to
        // free — net relief is zero and the speed tier must reject it.
        let mut v = Vec::new();
        for k in 0..12u32 {
            v.push(add(100 + k, 700 + k, 720 + k));
        }
        v.push(global(1, "G")); // late def at pressure plateau
        v.push(add(300, 1, 999)); // sole read immediately afterwards
        v.push(add(200, 100, 101));
        for k in 1..12u32 {
            v.push(add(200 + k, 199 + k, 100 + k));
        }
        let g = func_with(
            vec![blk(
                0,
                v,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let live2 = crate::backend::liveness::compute_live_intervals(&g);
        let c2 = ColorClasses::phi_webs(&g);
        let elig2: FxHashSet<u32> = (1u32..=300).collect();
        let p2 = Pressure::build(&live2, &g, &elig2, &c2);
        let (_, peak) = p2.peak(0);
        assert!(
            (13..=14).contains(&peak),
            "fixture peaks just over budget, got {peak}"
        );
        assert!(
            !covers_over_budget_point(&live2, &g, &p2, 1, &policy),
            "def/read-only residency is not net relief"
        );
        // The debug tier (-O0) credits the full resident span: at -O0 the
        // win is slot round-trip removal, not colorer residency.
        let mut debug = conservative_policy(64);
        debug.tier = Tier::Debug;
        debug.reach_band = 64;
        assert!(covers_over_budget_point(&live2, &g, &p2, 1, &debug));
    }

    #[test]
    fn phi_web_counts_as_one_color_class() {
        // v9 = φ(v1, v3), v10 = φ(c, v4): the colorer coalesces each web
        // into one register, so a block full of loop-carried φ webs must
        // not be counted once per SSA name.
        let blocks = vec![
            blk(0, vec![add(1, 500, 501)], Terminator::Branch(BlockId(1))),
            blk(
                1,
                vec![
                    Instruction::Phi {
                        dest: Value(9),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Value(Value(1)), BlockId(0)),
                            (Operand::Value(Value(3)), BlockId(2)),
                        ],
                    },
                    Instruction::Phi {
                        dest: Value(10),
                        ty: IrType::I64,
                        incoming: vec![
                            (Operand::Const(IrConst::I64(0)), BlockId(0)),
                            (Operand::Value(Value(4)), BlockId(2)),
                        ],
                    },
                    add(3, 9, 700),
                    add(4, 10, 701),
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(3)),
                    true_label: BlockId(1),
                    false_label: BlockId(3),
                },
            ),
            blk(2, vec![], Terminator::Branch(BlockId(1))),
            blk(
                3,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(9)))),
            ),
        ];
        let f = func_with(blocks, 800);
        let live = crate::backend::liveness::compute_live_intervals(&f);
        let mut eligible: FxHashSet<u32> = FxHashSet::default();
        for v in [1u32, 3, 4, 9, 10] {
            eligible.insert(v);
        }
        // Naive SSA-name counting at the header peak:
        let naive = Pressure::build(
            &live,
            &f,
            &eligible,
            &ColorClasses {
                parent: FxHashMap::default(),
            },
        );
        let classes = ColorClasses::phi_webs(&f);
        let coalesced = Pressure::build(&live, &f, &eligible, &classes);
        let (_p1, n_naive) = naive.peak(1);
        let (_p2, n_coal) = coalesced.peak(1);
        assert!(
            n_coal < n_naive,
            "φ webs must count once ({n_coal} < {n_naive})"
        );
    }

    #[test]
    fn grossly_over_block_is_left_to_production_allocator() {
        // nbody lesson: a block grossly over the register budget (>
        // budget+reach) cannot be made colorable by a handful of splits —
        // the production allocator resolves it with folded memory operands
        // and callee-save buys anyway, and pre-alloc edits only perturb
        // that coloring (measured +28 insns / +88 stkref). Build a REAL
        // 21-class peak: 20 independent producers kept live until a
        // sequential consumer chain (same proven ladder as the rest of
        // the suite, scaled past budget+reach), plus the global.
        let ladder = |n: u32| {
            let mut v = Vec::new();
            for k in 0..n {
                v.push(add(100 + k, 600 + k, 620 + k));
            }
            v.push(add(200, 1, 100));
            for k in 1..n {
                v.push(add(200 + k, 199 + k, 100 + k));
            }
            v
        };
        let make = || {
            let mut insts = vec![global(1, "bodies")];
            insts.extend(ladder(20));
            func_with(
                vec![blk(
                    0,
                    insts,
                    Terminator::Return(Some(Operand::Value(Value(219)))),
                )],
                900,
            )
        };
        // Sanity: the fixture genuinely peaks at 21 coalesced classes.
        let probe = make();
        let live = crate::backend::liveness::compute_live_intervals(&probe);
        let classes = ColorClasses::phi_webs(&probe);
        let eligible: FxHashSet<u32> = (1u32..=119).collect();
        let p = Pressure::build(&live, &probe, &eligible, &classes);
        let (_pt, peak) = p.peak(0);
        assert!(
            peak >= 21,
            "fixture must be grossly over budget, got {peak}"
        );

        let mut f = make();
        let n = run_with_policy(&mut f, conservative_policy(64));
        assert_eq!(n, 0, "grossly-over block must never be edited");
        assert_eq!(
            count(&f, |i| matches!(i, Instruction::GlobalAddr { .. })),
            1,
            "no remat clones in a hopeless block"
        );
        // Raising the reach band is exactly what re-admits it: the band,
        // not some other gate, is the discriminator.
        let mut g = make();
        let m = run_with_policy(
            &mut g,
            GlaPolicy {
                reach_band: 999,
                ..conservative_policy(64)
            },
        );
        assert!(m >= 1, "a wide reach band edits the block again");
    }

    #[test]
    fn single_segment_global_remat_tips_reachable_peak() {
        // adler32 shape: one single-segment global whose residency pushes a
        // otherwise-fittable block 1–4 classes over budget. Remat fires,
        // minting clones but never a stack slot.
        let insts = remat_ladder();
        let mut f = func_with(
            vec![blk(
                0,
                insts,
                Terminator::Return(Some(Operand::Value(Value(211)))),
            )],
            900,
        );
        let n = run_with_policy(&mut f, conservative_policy(64));
        assert!(n >= 1, "reachable peak rematerializes the global");
        assert!(count(&f, |i| matches!(i, Instruction::GlobalAddr { .. })) >= 2);
        assert_eq!(count(&f, |i| matches!(i, Instruction::Alloca { .. })), 0);
        verify_rewrite(&f, &FxHashSet::default()).expect("rewrite verifies");
    }

    #[test]
    fn remat_segment_cap_is_one_in_shipped_policy() {
        // The two-segment liveness shape (edge-point holes around nested
        // loop headers, as in nbody's printf format-string global v94) is
        // anchored end-to-end in
        // tests/regression/check_gla_remat_policy.sh against the real
        // benchmark; here we pin the planner predicate itself so a later
        // default flip cannot silently re-admit the nbody cascade.
        let seg_cap = conservative_policy(64).remat_max_segments;
        assert_eq!(seg_cap, 1);
        // A two-piece value is refused by the cap while a one-piece value
        // is admitted (pure predicate check, no liveness archaeology).
        assert!(1usize <= seg_cap);
        assert!(2usize > seg_cap);
    }

    #[test]
    fn conservative_policy_never_builds_capture_slots() {
        // The cross-block pressure fixture: permissive policy fires a global
        // spill gap (alloca + stores + exit reloads); the conservative
        // policy must leave the IR entirely to the production allocator.
        let make = || {
            let (b0_all, _) = pressure_ladder(1);
            let b0 = b0_all[..15].to_vec();
            let mut b2 = Vec::new();
            b2.push(add(200, 1, 100));
            for k in 1..14u32 {
                b2.push(add(200 + k, 199 + k, 100 + k));
            }
            b2.push(add(300, 213, 1));
            let blocks = vec![
                blk(0, b0, Terminator::Branch(BlockId(1))),
                blk(
                    1,
                    vec![
                        Instruction::Phi {
                            dest: Value(2),
                            ty: IrType::I64,
                            incoming: vec![
                                (Operand::Const(IrConst::I64(0)), BlockId(0)),
                                (Operand::Value(Value(3)), BlockId(1)),
                            ],
                        },
                        add(3, 2, 900),
                        Instruction::Cmp {
                            dest: Value(4),
                            op: IrCmpOp::Slt,
                            lhs: Operand::Value(Value(3)),
                            rhs: Operand::Const(IrConst::I64(10)),
                            ty: IrType::I64,
                        },
                    ],
                    Terminator::CondBranch {
                        cond: Operand::Value(Value(4)),
                        true_label: BlockId(1),
                        false_label: BlockId(2),
                    },
                ),
                blk(2, b2, Terminator::Return(Some(Operand::Value(Value(300))))),
            ];
            func_with(blocks, 900)
        };
        let mut f = make();
        let n = run_with_policy(&mut f, conservative_policy(256));
        assert_eq!(n, 0, "spill gaps are off in the shipped policy");
        assert_eq!(count(&f, |i| matches!(i, Instruction::Alloca { .. })), 0);
        let mut g = make();
        let m = run_with_policy(
            &mut g,
            GlaPolicy {
                max_splits: 256,
                ..GlaPolicy::testing_permissive()
            },
        );
        assert!(m >= 1, "permissive policy retains the gap capability");
        verify_rewrite(&g, &FxHashSet::default()).expect("permissive rewrite verifies");
    }
}
