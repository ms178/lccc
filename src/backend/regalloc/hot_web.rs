//! Globally priced, whole-web callee-saved eviction for x86 loop state.
//! Run before segment fill; never evict another loop web or an ABI hint.
use super::*;

pub(super) struct HotWebSteal<'a> {
    pub config: &'a RegAllocConfig,
    pub phi_pairs: &'a [PhiCoalesceCandidate],
    pub eligible: &'a FxHashSet<u32>,
    pub iv_map: &'a FxHashMap<u32, (u32, u32)>,
    pub merged_of: &'a FxHashMap<u32, LiveInterval>,
    pub coalesce_groups: &'a FxHashMap<u32, Vec<u32>>,
    pub coalesce_member_of: &'a FxHashMap<u32, u32>,
    pub use_count: &'a FxHashMap<u32, u64>,
}

pub(super) fn steal_hot_loop_webs(
    assignments: &mut FxHashMap<u32, PhysReg>,
    context: &HotWebSteal<'_>,
) -> usize {
    let HotWebSteal {
        config,
        phi_pairs,
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
        .collect();
    let mut candidates: Vec<(u32, u64)> = phi_pairs
        .iter()
        .map(|c| c.phi_dest)
        .filter(|v| eligible.contains(v))
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
