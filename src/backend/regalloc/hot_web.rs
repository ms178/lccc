//! Globally priced, whole-web callee-saved eviction for x86 loop state.
//! Run before segment fill; never evict another loop web or an ABI hint.
use super::*;

pub(super) struct HotWebSteal<'a> {
    pub config: &'a RegAllocConfig,
    pub phi_pairs: &'a [PhiCoalesceCandidate],
    pub loop_carried: &'a FxHashSet<u32>,
    pub eligible: &'a FxHashSet<u32>,
    pub iv_map: &'a FxHashMap<u32, (u32, u32)>,
    pub merged_of: &'a FxHashMap<u32, LiveInterval>,
    pub coalesce_groups: &'a FxHashMap<u32, Vec<u32>>,
    pub coalesce_member_of: &'a FxHashMap<u32, u32>,
    pub use_count: &'a FxHashMap<u32, u64>,
}

/// Profitability prior for the additional noncoalescible-state policy. Keep
/// ordinary loops on their established allocator path: an eight-bit masked
/// equality suggests a sparse arm, not a semantic fact about the inputs.
pub(super) fn has_sparse_mask_comparison(func: &IrFunction) -> bool {
    use crate::ir::reexports::IrCmpOp;
    let defs: FxHashMap<_, _> = func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|inst| inst.dest().map(|dest| (dest.0, inst)))
        .collect();
    func.blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|inst| {
            let Instruction::Cmp {
                op: IrCmpOp::Eq | IrCmpOp::Ne,
                lhs,
                rhs,
                ..
            } = inst
            else {
                return false;
            };
            let (value, constant) = match (lhs, rhs) {
                (Operand::Value(v), Operand::Const(c)) | (Operand::Const(c), Operand::Value(v)) => {
                    (v, c)
                }
                _ => return false,
            };
            let Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs,
                rhs,
                ty,
                ..
            }) = defs.get(&value.0)
            else {
                return false;
            };
            if !ty.is_integer() || ty.size() == 0 || ty.size() > 8 {
                return false;
            }
            let mask = match (lhs, rhs) {
                (Operand::Const(c), _) | (_, Operand::Const(c)) => c.to_i64(),
                _ => None,
            };
            match (mask, constant.to_i64()) {
                (Some(mask), Some(c)) => {
                    let mask = mask as u64 & (u64::MAX >> (64 - ty.size() * 8));
                    mask.count_ones() >= 8 && c as u64 & !mask == 0
                }
                _ => false,
            }
        })
}

/// Allocation promotion is not destructive phi coalescing. In particular,
/// a loop header fed by a conditional (multi-definition) merge cannot safely
/// coalesce with that source, but may still keep its own callee-saved home.
/// Admit only multiply defined copy destinations updated in a loop whose
/// allocation interval covers a complete natural-loop extent. This adds no
/// new register-sharing relation and never changes the coalescing legality gate.
pub(super) fn loop_carried_copy_dests(
    func: &IrFunction,
    liveness: &LivenessResult,
) -> FxHashSet<u32> {
    let mut definitions = FxHashMap::<u32, usize>::default();
    for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
        if let Some(dest) = inst.dest() {
            *definitions.entry(dest.0).or_default() += 1;
        }
    }
    let covering: FxHashSet<u32> = liveness
        .intervals
        .iter()
        .filter(|iv| {
            liveness
                .loop_extents
                .iter()
                .any(|&(start, end)| iv.start <= start && iv.end >= end)
        })
        .map(|iv| iv.value_id)
        .collect();
    func.blocks
        .iter()
        .enumerate()
        .filter(|(b, _)| liveness.block_loop_depth.get(*b).copied().unwrap_or(0) != 0)
        .flat_map(|(_, b)| &b.instructions)
        .filter_map(|inst| {
            let Instruction::Copy {
                dest,
                src: Operand::Value(_),
                ..
            } = inst
            else {
                return None;
            };
            (definitions.get(&dest.0).copied().unwrap_or(0) > 1 && covering.contains(&dest.0))
                .then_some(dest.0)
        })
        .collect()
}

pub(super) fn steal_hot_loop_webs(
    assignments: &mut FxHashMap<u32, PhysReg>,
    context: &HotWebSteal<'_>,
) -> usize {
    let HotWebSteal {
        config,
        phi_pairs,
        loop_carried,
        eligible,
        iv_map,
        merged_of,
        coalesce_groups,
        coalesce_member_of,
        use_count,
    } = *context;
    if config.ra_config.no_hot_web_steal {
        return 0;
    }
    let k = config.ra_config.hot_web_steal;
    // Web-aware use count: the leader's own count plus every coalesce
    // member's (the state-machine webs the phi-coalesce machinery
    // propagates homes through).
    let web_use_count = |v: u32| -> u64 {
        let mut uc = use_count.get(&v).copied().unwrap_or(0);
        if let Some(members) = coalesce_groups.get(&v) {
            uc = uc.max(summed_use_weight(members, use_count));
        }
        uc
    };
    let phi_pair_values: FxHashSet<u32> = phi_pairs
        .iter()
        .flat_map(|c| [c.phi_dest, c.backedge_src])
        .chain(loop_carried.iter().copied())
        .collect();
    let mut candidates: Vec<(u32, u64)> = phi_pairs
        .iter()
        .map(|c| c.phi_dest)
        .chain(loop_carried.iter().copied())
        .filter(|v| eligible.contains(v) && !config.reg_hints.contains_key(v))
        .filter(|v| !coalesce_member_of.contains_key(v))
        .filter(|v| iv_map.get(v).is_some_and(|&(s, e)| e > s))
        .map(|v| (v, web_use_count(v)))
        .filter(|&(_, count)| count >= 10)
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    candidates.dedup_by_key(|&mut (v, _)| v);

    let overlaps_vid = |a: u32, b: u32| -> bool {
        match (
            allocation_owner_bounds(a, merged_of, iv_map),
            allocation_owner_bounds(b, merged_of, iv_map),
        ) {
            (Some(ia), Some(ib)) => intervals_overlap(ia, ib),
            _ => false,
        }
    };
    let mut holders_by_reg: FxHashMap<u8, Vec<u32>> = FxHashMap::default();
    for (&v, &r) in assignments.iter() {
        holders_by_reg.entry(r.0).or_default().push(v);
    }

    let mut steals = 0;
    for &(vid, hot_count) in &candidates {
        if steals >= k || assignments.contains_key(&vid) {
            continue;
        }
        let mut best: Option<(u8, Vec<u32>, u64)> = None;
        for (&reg_id, holders) in &holders_by_reg {
            if !config.available_regs.iter().any(|r| r.0 == reg_id) {
                continue;
            }
            if holders.iter().any(|h| {
                phi_pair_values.contains(h)
                    || config.reg_hints.contains_key(h)
                    || allocation_owner_bounds(*h, merged_of, iv_map).is_none()
            }) {
                continue;
            }
            let evict: Vec<u32> = holders
                .iter()
                .copied()
                .filter(|&h| overlaps_vid(h, vid))
                .collect();
            if evict.is_empty() {
                continue;
            }
            // evict_group removes WHOLE copy webs, including members
            // outside the incoming span. Price that exact mutation, not
            // just the overlapping subset (which undercounts cold leaders
            // whose nonoverlapping members are hot).
            let cost =
                hot_web_eviction_cost(&evict, coalesce_member_of, coalesce_groups, use_count);
            if cost >= hot_count {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|&(best_reg, _, c)| (cost, reg_id) < (c, best_reg))
            {
                best = Some((reg_id, evict, cost));
            }
        }
        if let Some((reg_id, evict, _)) = best {
            for &v in &evict {
                evict_group(
                    assignments,
                    &mut holders_by_reg,
                    v,
                    coalesce_member_of,
                    coalesce_groups,
                );
            }
            assignments.insert(vid, PhysReg(reg_id));
            holders_by_reg.entry(reg_id).or_default().push(vid);
            steals += 1;
        }
    }
    propagate_coalesce_members(assignments, coalesce_member_of);
    steals
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        config: RegAllocConfig,
        assignments: FxHashMap<u32, PhysReg>,
        pairs: Vec<PhiCoalesceCandidate>,
        loop_carried: FxHashSet<u32>,
        eligible: FxHashSet<u32>,
        bounds: FxHashMap<u32, (u32, u32)>,
        merged: FxHashMap<u32, LiveInterval>,
        groups: FxHashMap<u32, Vec<u32>>,
        members: FxHashMap<u32, u32>,
        weights: FxHashMap<u32, u64>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                config: RegAllocConfig {
                    available_regs: vec![PhysReg(1), PhysReg(2)],
                    caller_saved_regs: vec![PhysReg(10)],
                    call_arg_regs: vec![],
                    indirect_target_regs: vec![],
                    accumulator_policy: AccumulatorPolicy {
                        operand_order: AccumulatorOperandOrder::LhsFirst,
                        return_consumes_accumulator: false,
                    },
                    allow_inline_asm_regalloc: false,
                    xmm_regs: vec![],
                    never_materialized: FxHashSet::default(),
                    folded_index_uses: FxHashMap::default(),
                    reg_hints: FxHashMap::default(),
                    ra_config: Arc::new(RaConfig::default()),
                    leaf_caller_saved_homes: false,
                },
                assignments: FxHashMap::default(),
                pairs: vec![],
                loop_carried: FxHashSet::default(),
                eligible: FxHashSet::default(),
                bounds: FxHashMap::default(),
                merged: FxHashMap::default(),
                groups: FxHashMap::default(),
                members: FxHashMap::default(),
                weights: FxHashMap::default(),
            }
        }

        fn holder(&mut self, v: u32, r: u8, span: (u32, u32), cost: u64) {
            self.assignments.insert(v, PhysReg(r));
            self.bounds.insert(v, span);
            self.weights.insert(v, cost);
        }

        fn candidate(&mut self, v: u32, span: (u32, u32), cost: u64) {
            self.pairs.push(PhiCoalesceCandidate {
                phi_dest: v,
                backedge_src: v + 100,
                block_idx: 1,
                source_block_idx: 1,
                source_def_idx: 0,
                copy_idx: 1,
            });
            self.eligible.insert(v);
            self.bounds.insert(v, span);
            self.weights.insert(v, cost);
        }

        fn run(&mut self) -> usize {
            steal_hot_loop_webs(
                &mut self.assignments,
                &HotWebSteal {
                    config: &self.config,
                    phi_pairs: &self.pairs,
                    loop_carried: &self.loop_carried,
                    eligible: &self.eligible,
                    iv_map: &self.bounds,
                    merged_of: &self.merged,
                    coalesce_groups: &self.groups,
                    coalesce_member_of: &self.members,
                    use_count: &self.weights,
                },
            )
        }
    }

    #[test]
    fn extra_loop_state_policy_requires_a_sparse_integer_comparison() {
        use crate::ir::reexports::{BasicBlock, BlockId, IrCmpOp, Value};
        let mut f = IrFunction::new("sparse_state".into(), IrType::U32, vec![], false);
        for (mask, constant, expected) in [(255, 0, true), (127, 0, false), (255, 256, false)] {
            f.blocks = vec![BasicBlock {
                label: BlockId(0),
                source_spans: vec![],
                instructions: vec![
                    Instruction::BinOp {
                        dest: Value(1),
                        op: IrBinOp::And,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(mask)),
                        ty: IrType::U32,
                    },
                    Instruction::Cmp {
                        dest: Value(2),
                        op: IrCmpOp::Eq,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Const(IrConst::I32(constant)),
                        ty: IrType::U32,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
            }];
            assert_eq!(has_sparse_mask_comparison(&f), expected);
        }
        f.blocks[0].instructions.pop();
        assert!(
            !has_sparse_mask_comparison(&f),
            "a masked value alone is not a branch prior"
        );
    }

    #[test]
    fn hot_web_promotes_loop_state_without_destructive_coalescing() {
        let mut f = Fixture::new();
        f.holder(1, 1, (0, 100), 10);
        f.candidate(10, (20, 80), 40);
        f.pairs.clear(); // Its multi-definition backedge cannot be coalesced.
        f.loop_carried.insert(10);
        assert_eq!(f.run(), 1);
        assert_eq!(f.assignments.get(&10), Some(&PhysReg(1)));
        assert!(!f.assignments.contains_key(&1));
        assert!(f.groups.is_empty() && f.members.is_empty());
    }

    #[test]
    fn hot_web_does_not_promote_an_abi_hinted_candidate() {
        let mut f = Fixture::new();
        f.holder(1, 1, (0, 100), 1);
        f.candidate(10, (20, 80), 40);
        f.loop_carried.insert(10);
        f.config.reg_hints.insert(10, PhysReg(10));
        assert_eq!(f.run(), 0);
    }

    #[test]
    fn hot_web_preserves_noncoalescible_loop_state_holders() {
        let mut f = Fixture::new();
        f.holder(1, 1, (0, 100), 1);
        f.loop_carried.insert(1);
        f.candidate(10, (20, 80), 40);
        assert_eq!(f.run(), 0);
        assert_eq!(f.assignments.get(&1), Some(&PhysReg(1)));
    }

    #[test]
    fn loop_state_discovery_separates_header_state_from_conditional_merges() {
        use crate::ir::reexports::{BasicBlock, BlockId, IrCmpOp, Value};
        let copy = |dest, src| Instruction::Copy {
            dest: Value(dest),
            src,
        };
        let branch = |target| Terminator::Branch(BlockId(target));
        let block = |label, instructions, terminator| BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
        };
        let mut func = IrFunction::new("conditional_recurrence".into(), IrType::U64, vec![], false);
        func.blocks = vec![
            block(
                0,
                vec![
                    Instruction::ParamRef {
                        dest: Value(0),
                        param_idx: 0,
                        ty: IrType::U64,
                    },
                    copy(10, Operand::Const(IrConst::I64(0))),
                ],
                branch(1),
            ),
            block(
                1,
                vec![
                    Instruction::BinOp {
                        dest: Value(11),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(10)),
                        rhs: Operand::Const(IrConst::I64(1)),
                        ty: IrType::U64,
                    },
                    Instruction::Cmp {
                        dest: Value(12),
                        op: IrCmpOp::Ult,
                        lhs: Operand::Value(Value(11)),
                        rhs: Operand::Value(Value(0)),
                        ty: IrType::U64,
                    },
                ],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(12)),
                    true_label: BlockId(2),
                    false_label: BlockId(3),
                },
            ),
            block(2, vec![copy(20, Operand::Value(Value(11)))], branch(4)),
            block(3, vec![copy(20, Operand::Value(Value(0)))], branch(4)),
            block(
                4,
                vec![copy(10, Operand::Value(Value(20)))],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(12)),
                    true_label: BlockId(1),
                    false_label: BlockId(5),
                },
            ),
            block(
                5,
                vec![],
                Terminator::Return(Some(Operand::Value(Value(10)))),
            ),
        ];
        func.next_value_id = 21;
        let live = compute_live_intervals(&func);
        assert_eq!(
            loop_carried_copy_dests(&func, &live),
            FxHashSet::from_iter([10])
        );
        assert!(
            !detect_phi_coalesce_groups_with_config(&func, &live, &RaConfig::default())
                .iter()
                .any(|pair| pair.phi_dest == 10)
        );
    }

    #[test]
    fn hot_web_steals_cold_home_and_preserves_nonoverlapping_holder() {
        let mut f = Fixture::new();
        f.holder(1, 1, (0, 100), 2);
        f.holder(2, 1, (100, 110), 50);
        f.candidate(10, (20, 100), 30);
        assert_eq!(f.run(), 1);
        assert_eq!(f.assignments.get(&10), Some(&PhysReg(1)));
        assert!(!f.assignments.contains_key(&1));
        assert_eq!(f.assignments.get(&2), Some(&PhysReg(1)));
    }

    #[test]
    fn hot_web_sums_all_overlapping_holders_and_requires_strict_profit() {
        for incoming in [12, 13] {
            let mut f = Fixture::new();
            f.holder(1, 1, (0, 40), 6);
            f.holder(2, 1, (40, 100), 6);
            f.candidate(10, (20, 80), incoming);
            assert_eq!(f.run(), usize::from(incoming > 12));
        }
    }

    #[test]
    fn hot_web_prices_and_evicts_complete_copy_group_without_double_counting() {
        for incoming in [90, 101] {
            let mut f = Fixture::new();
            f.holder(1, 1, (0, 20), 1);
            f.holder(2, 1, (30, 100), 99);
            f.groups.insert(1, vec![1, 2]);
            f.members.insert(2, 1);
            f.merged.insert(
                1,
                LiveInterval {
                    value_id: 1,
                    start: 0,
                    end: 100,
                },
            );
            f.candidate(10, (5, 15), incoming);
            assert_eq!(f.run(), usize::from(incoming > 100));
            if incoming > 100 {
                assert!(!f.assignments.contains_key(&1));
                assert!(!f.assignments.contains_key(&2));
            }
        }
        assert_eq!(
            hot_web_eviction_cost(
                &[1, 2, 1],
                &FxHashMap::from_iter([(2, 1)]),
                &FxHashMap::from_iter([(1, vec![1, 2])]),
                &FxHashMap::from_iter([(1, 1), (2, 99)])
            ),
            100
        );
    }

    #[test]
    fn hot_web_never_displaces_loop_pair_or_abi_hint() {
        for protected in [1, 101, 2] {
            let mut f = Fixture::new();
            f.candidate(1, (0, 100), 10);
            f.candidate(10, (20, 80), 1000);
            f.holder(protected, 1, (0, 100), 1);
            if protected == 2 {
                f.config.reg_hints.insert(2, PhysReg(1));
            }
            assert_eq!(f.run(), 0);
            assert!(!f.assignments.contains_key(&10));
        }
    }

    #[test]
    fn hot_web_refuses_unknown_coverage_caller_saved_and_reserved_homes() {
        for variant in 0..3 {
            let mut f = Fixture::new();
            f.holder(
                1,
                if variant == 1 {
                    10
                } else if variant == 2 {
                    6
                } else {
                    1
                },
                (0, 100),
                1,
            );
            if variant == 0 {
                f.bounds.remove(&1);
            }
            f.candidate(10, (20, 80), 100);
            assert_eq!(f.run(), 0);
        }
    }

    #[test]
    fn hot_web_is_ranked_budgeted_and_disableable() {
        for limit in [0, 1, 2] {
            let mut f = Fixture::new();
            Arc::make_mut(&mut f.config.ra_config).hot_web_steal = limit;
            f.holder(1, 1, (0, 100), 1);
            f.holder(2, 2, (0, 100), 2);
            f.candidate(10, (20, 80), 20);
            f.candidate(11, (20, 80), 40);
            assert_eq!(f.run(), limit);
            if limit > 0 {
                assert_eq!(f.assignments.get(&11), Some(&PhysReg(1)));
            }
        }
        let mut f = Fixture::new();
        f.holder(1, 1, (0, 100), 1);
        f.candidate(10, (20, 80), 20);
        Arc::make_mut(&mut f.config.ra_config).no_hot_web_steal = true;
        assert_eq!(f.run(), 0);
    }

    #[test]
    fn hot_web_uses_merged_candidate_extent_and_whole_web_heat() {
        let mut f = Fixture::new();
        f.holder(1, 1, (40, 80), 10);
        f.candidate(10, (0, 10), 1);
        f.bounds.insert(11, (20, 90));
        f.weights.insert(11, 30);
        f.groups.insert(10, vec![10, 11]);
        f.members.insert(11, 10);
        f.merged.insert(
            10,
            LiveInterval {
                value_id: 10,
                start: 0,
                end: 90,
            },
        );
        assert_eq!(f.run(), 1);
        assert_eq!(f.assignments.get(&11), Some(&PhysReg(1)));
    }

    #[test]
    fn hot_web_rejects_members_degenerate_cold_and_ineligible_candidates() {
        for variant in 0..4 {
            let mut f = Fixture::new();
            f.holder(1, 1, (0, 100), 1);
            f.candidate(10, (20, 80), 20);
            match variant {
                0 => {
                    f.members.insert(10, 11);
                }
                1 => {
                    f.bounds.insert(10, (20, 20));
                }
                2 => {
                    f.weights.insert(10, 9);
                }
                _ => {
                    f.eligible.remove(&10);
                }
            }
            assert_eq!(f.run(), 0);
        }
    }

    #[test]
    fn hot_web_tie_breaks_by_register_and_value_id() {
        for reverse in [false, true] {
            let mut f = Fixture::new();
            for v in if reverse { [2, 1] } else { [1, 2] } {
                f.holder(v, v as u8, (0, 100), 1);
            }
            for v in if reverse { [11, 10] } else { [10, 11] } {
                f.candidate(v, (20, 80), 30);
            }
            assert_eq!(f.run(), 2);
            assert_eq!(f.assignments.get(&10), Some(&PhysReg(1)));
            assert_eq!(f.assignments.get(&11), Some(&PhysReg(2)));
        }
    }

    #[test]
    fn hot_web_overflow_is_not_false_profit() {
        let mut f = Fixture::new();
        f.holder(1, 1, (0, 50), u64::MAX);
        f.holder(2, 1, (50, 100), 1);
        f.candidate(10, (20, 80), u64::MAX);
        assert_eq!(f.run(), 0);
    }
}
