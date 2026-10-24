//! Pressure-guided live-range shrinkage on SSA IR.
//!
//! GCC's `-flive-range-shrinkage` is a pre-RA, intra-block scheduler: among
//! ready instructions it prefers a negative *excess-pressure* change and then
//! preserves source order.  This pass uses the same sound starting point, but
//! adds two guards that matter in an IR pipeline with many later transforms:
//!
//! * scheduling is a stable list-scheduling problem over an explicit SSA DAG;
//!   memory operations, calls, atomics, asm, stack manipulation, PHIs and
//!   target-ABI pseudo operations are hard region boundaries;
//! * a candidate schedule is committed only when the measured pressure model
//!   improves excess pressure without increasing total live area or peak in
//!   either register class.  A heuristic that merely looks profitable locally
//!   therefore cannot make the function's modelled allocation problem worse.
//!
//! This is deliberately not a general latency scheduler.  Moving work below
//! the register-file pressure threshold only perturbs debug locations and can
//! hide instruction-level parallelism, so such regions retain their original
//! order.  GP and vector/FP values have independent budgets; 64-bit integer
//! values on a 32-bit target cost two GP registers.  The budgets describe the
//! architectural register files, not a particular host CPU, and are checked
//! against the final IR after all ordinary middle-end transforms.

use crate::backend::Target;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrConst, IrFunction, IrModule, Operand, Value};

const GP: usize = 0;
const VECTOR: usize = 1;
const CLASSES: usize = 2;

/// Register-file budgets used by the excess-pressure objective.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PressureBudget {
    pub gp: u32,
    pub vector: u32,
}

impl PressureBudget {
    fn for_target(target: Target) -> Self {
        match target {
            // RSP and RBP are not general allocation candidates.  Keep the
            // legacy budget under APX until every IR allocator path admits
            // EGPRs; the late MachInst windows already use them independently.
            Target::X86_64 => Self { gp: 14, vector: 16 },
            Target::I686 => Self { gp: 6, vector: 8 },
            Target::Aarch64 => Self { gp: 28, vector: 32 },
            Target::Riscv64 => Self { gp: 27, vector: 32 },
        }
    }

    fn get(self, class: usize) -> u32 {
        match class {
            GP => self.gp,
            VECTOR => self.vector,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ClassMetrics {
    live_area: u64,
    live_peak: u32,
    excess_area: u64,
    excess_peak: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Metrics([ClassMetrics; CLASSES]);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ShrinkStats {
    pub regions: usize,
    pub high_pressure_regions: usize,
    pub changed_regions: usize,
    pub excess_area_before: u64,
    pub excess_area_after: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ValueInfo {
    class: usize,
    weight: u16,
}

#[derive(Debug)]
struct Node {
    original: usize,
    uses: Vec<u32>,
    dest: Option<u32>,
    successors: Vec<usize>,
    remaining_predecessors: usize,
}

#[derive(Clone)]
struct Simulation {
    remaining: Vec<u32>,
    live: Vec<u16>,
    current: [u32; CLASSES],
    metrics: Metrics,
}

impl Simulation {
    fn new(
        remaining: Vec<u32>,
        relevant: &[u32],
        locally_defined: &FxHashSet<u32>,
        info: &[ValueInfo],
    ) -> Self {
        let mut sim = Self {
            remaining,
            live: vec![0; info.len()],
            current: [0; CLASSES],
            metrics: Metrics::default(),
        };
        for &v in relevant {
            let vi = info[v as usize];
            if sim.remaining[v as usize] != 0 && !locally_defined.contains(&v) {
                sim.live[v as usize] = vi.weight;
                sim.current[vi.class] += u32::from(vi.weight);
            }
        }
        sim
    }

    fn pressure_delta(&self, node: &Node, info: &[ValueInfo]) -> ([i32; CLASSES], u32, u32) {
        let mut deaths = [0i32; CLASSES];
        let mut births = [0i32; CLASSES];
        let mut death_weight = 0u32;
        let mut birth_weight = 0u32;
        for &v in &node.uses {
            let i = v as usize;
            if self.remaining[i] == 1 && self.live[i] != 0 {
                let vi = info[i];
                deaths[vi.class] -= i32::from(vi.weight);
                death_weight += u32::from(vi.weight);
            }
        }
        if let Some(v) = node.dest {
            let i = v as usize;
            if self.remaining[i] != 0 && self.live[i] == 0 {
                let vi = info[i];
                births[vi.class] += i32::from(vi.weight);
                birth_weight += u32::from(vi.weight);
            }
        }
        let mut delta = [0i32; CLASSES];
        for class in 0..CLASSES {
            delta[class] = deaths[class] + births[class];
        }
        (delta, death_weight, birth_weight)
    }

    fn excess_after(&self, node: &Node, info: &[ValueInfo], budget: PressureBudget) -> (u32, u32) {
        let (delta, _, _) = self.pressure_delta(node, info);
        let mut total = 0u32;
        let mut worst = 0u32;
        for class in 0..CLASSES {
            let next = (self.current[class] as i32 + delta[class]).max(0) as u32;
            let excess = next.saturating_sub(budget.get(class));
            total = total.saturating_add(excess);
            worst = worst.max(excess);
        }
        (total, worst)
    }

    fn apply(&mut self, node: &Node, info: &[ValueInfo], budget: PressureBudget) {
        // SSA uses precede the definition at the same program point.  This is
        // also the allocator's die-at-birth contract in backend::live_range.
        for &v in &node.uses {
            let i = v as usize;
            debug_assert!(self.remaining[i] != 0, "use count underflow at v{v}");
            self.remaining[i] = self.remaining[i].saturating_sub(1);
            if self.remaining[i] == 0 && self.live[i] != 0 {
                let vi = info[i];
                self.current[vi.class] =
                    self.current[vi.class].saturating_sub(u32::from(vi.weight));
                self.live[i] = 0;
            }
        }
        if let Some(v) = node.dest {
            let i = v as usize;
            if self.remaining[i] != 0 && self.live[i] == 0 {
                let vi = info[i];
                self.live[i] = vi.weight;
                self.current[vi.class] += u32::from(vi.weight);
            }
        }
        for class in 0..CLASSES {
            let live = self.current[class];
            let excess = live.saturating_sub(budget.get(class));
            let m = &mut self.metrics.0[class];
            m.live_area = m.live_area.saturating_add(u64::from(live));
            m.live_peak = m.live_peak.max(live);
            m.excess_area = m.excess_area.saturating_add(u64::from(excess));
            m.excess_peak = m.excess_peak.max(excess);
        }
    }
}

/// Return whether an instruction can move inside a pure scheduling region.
/// Everything not explicitly named is a boundary.  In particular GlobalAddr
/// is not necessarily pure on targets where a dynamic TLS model emits a call.
fn schedulable(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Cmp { .. }
            | Instruction::Cast { .. }
            | Instruction::Copy { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::Select { .. }
    )
}

fn value_weight(ty: IrType, target: Target) -> ValueInfo {
    if ty == IrType::Void {
        return ValueInfo::default();
    }
    if ty.is_float() {
        return ValueInfo {
            class: VECTOR,
            weight: 1,
        };
    }
    let pointer = target.ptr_size().max(1);
    let bytes = ty.size().max(1);
    ValueInfo {
        class: GP,
        weight: bytes.div_ceil(pointer).clamp(1, 8) as u16,
    }
}

fn const_weight(c: IrConst, target: Target) -> ValueInfo {
    match c {
        IrConst::F32(_) => value_weight(IrType::F32, target),
        IrConst::F64(_) => value_weight(IrType::F64, target),
        IrConst::D32(_) => value_weight(IrType::F32, target),
        IrConst::D64(_) => value_weight(IrType::F64, target),
        IrConst::LongDouble(_, _) => ValueInfo {
            class: VECTOR,
            weight: 1,
        },
        IrConst::I8(_) => value_weight(IrType::I8, target),
        IrConst::I16(_) => value_weight(IrType::I16, target),
        IrConst::I32(_) | IrConst::Zero => value_weight(IrType::I32, target),
        IrConst::I64(_) => value_weight(IrType::I64, target),
        IrConst::I128(_) => ValueInfo {
            class: GP,
            weight: 16u16.div_ceil(target.ptr_size() as u16).max(1),
        },
    }
}

fn unique_instruction_uses(inst: &Instruction) -> Vec<u32> {
    let mut uses = Vec::new();
    inst.for_each_used_value(|v| uses.push(v));
    uses.sort_unstable();
    uses.dedup();
    uses
}

fn unique_terminator_uses(term: &crate::ir::reexports::Terminator) -> Vec<u32> {
    let mut uses = Vec::new();
    term.for_each_used_value(|v| uses.push(v));
    uses.sort_unstable();
    uses.dedup();
    uses
}

fn collect_function_facts(
    func: &IrFunction,
    target: Target,
) -> Option<(Vec<u32>, Vec<Option<(usize, usize)>>, Vec<ValueInfo>)> {
    let mut max_value = func.max_value_id() as usize;
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| max_value = max_value.max(v as usize));
        }
        block
            .terminator
            .for_each_used_value(|v| max_value = max_value.max(v as usize));
    }
    let len = max_value.saturating_add(1);
    let mut total_uses = vec![0u32; len];
    let mut definitions = vec![None; len];
    let mut info = vec![ValueInfo::default(); len];
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            for v in unique_instruction_uses(inst) {
                total_uses[v as usize] = total_uses[v as usize].saturating_add(1);
            }
            if let Some(dest) = inst.dest() {
                if definitions[dest.0 as usize].replace((bi, ii)).is_some() {
                    // Post-phi-coalescing webs are position dependent.  The
                    // driver runs this pass in SSA form; fail closed if a
                    // later pipeline change ever violates that contract.
                    return None;
                }
                if let Some(ty) = inst.result_type() {
                    info[dest.0 as usize] = value_weight(ty, target);
                }
            }
        }
        for v in unique_terminator_uses(&block.terminator) {
            total_uses[v as usize] = total_uses[v as usize].saturating_add(1);
        }
    }
    // Copy is the one ordinary SSA definition whose result_type is context
    // dependent.  Propagate carrier classes through copy chains to a fixpoint.
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let Instruction::Copy { dest, src } = inst else {
                    continue;
                };
                if info[dest.0 as usize].weight != 0 {
                    continue;
                }
                let inherited = match src {
                    Operand::Const(c) => const_weight(*c, target),
                    Operand::Value(v) => info[v.0 as usize],
                };
                if inherited.weight != 0 {
                    info[dest.0 as usize] = inherited;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    // Unknown carrier types are a profitability limitation, not a correctness
    // licence.  Treat them as one GP value rather than dropping them from the
    // pressure objective altogether.
    for entry in &mut info {
        if entry.weight == 0 {
            *entry = ValueInfo {
                class: GP,
                weight: 1,
            };
        }
    }
    Some((total_uses, definitions, info))
}

fn build_nodes(
    instructions: &[Instruction],
    start: usize,
    end: usize,
    info: &[ValueInfo],
) -> Option<(Vec<Node>, Vec<u32>)> {
    let mut local_def = FxHashMap::default();
    let mut relevant = FxHashSet::default();
    for (node, inst) in instructions[start..end].iter().enumerate() {
        if let Some(dest) = inst.dest() {
            local_def.insert(dest.0, node);
            relevant.insert(dest.0);
        }
        inst.for_each_used_value(|v| {
            relevant.insert(v);
        });
    }
    let mut nodes: Vec<Node> = instructions[start..end]
        .iter()
        .enumerate()
        .map(|(offset, inst)| {
            let mut uses = Vec::new();
            inst.for_each_used_value(|v| uses.push(v));
            uses.sort_unstable();
            uses.dedup();
            Node {
                original: start + offset,
                uses,
                dest: inst.dest().map(|d| d.0),
                successors: Vec::new(),
                remaining_predecessors: 0,
            }
        })
        .collect();
    let mut edges = FxHashSet::default();
    for node in 0..nodes.len() {
        let uses = nodes[node].uses.clone();
        for v in uses {
            if let Some(&producer) = local_def.get(&v) {
                if producer == node {
                    return None;
                }
                if edges.insert((producer, node)) {
                    nodes[producer].successors.push(node);
                    nodes[node].remaining_predecessors += 1;
                }
            }
        }
    }
    // The info slice is an explicit sanity input: every relevant id must be
    // representable before the simulator indexes it.
    if relevant.iter().any(|&v| v as usize >= info.len()) {
        return None;
    }
    let mut relevant: Vec<u32> = relevant.into_iter().collect();
    relevant.sort_unstable();
    Some((nodes, relevant))
}

fn simulate_original(
    nodes: &[Node],
    remaining: &[u32],
    relevant: &[u32],
    locally_defined: &FxHashSet<u32>,
    info: &[ValueInfo],
    budget: PressureBudget,
) -> Metrics {
    let mut sim = Simulation::new(remaining.to_vec(), relevant, locally_defined, info);
    for node in nodes {
        sim.apply(node, info, budget);
    }
    sim.metrics
}

fn candidate_order(
    nodes: &mut [Node],
    remaining: &[u32],
    relevant: &[u32],
    locally_defined: &FxHashSet<u32>,
    info: &[ValueInfo],
    budget: PressureBudget,
) -> Option<(Vec<usize>, Metrics)> {
    let mut sim = Simulation::new(remaining.to_vec(), relevant, locally_defined, info);
    let mut ready: Vec<usize> = (0..nodes.len())
        .filter(|&i| nodes[i].remaining_predecessors == 0)
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while !ready.is_empty() {
        let mut best: Option<(usize, (u32, u32, std::cmp::Reverse<u32>, usize))> = None;
        for (position, &node_index) in ready.iter().enumerate() {
            let node = &nodes[node_index];
            let (excess, worst) = sim.excess_after(node, info, budget);
            let (_, deaths, _) = sim.pressure_delta(node, info);
            // Absolute post-instruction excess is the primary key (the current
            // pressure is common to every candidate).  Prefer deaths on a tie,
            // then source order.  This retains GCC's stable behaviour below
            // the pressure threshold while avoiding its one-step-only choice
            // between equal total and equal worst-class excess.
            let key = (excess, worst, std::cmp::Reverse(deaths), node.original);
            if best.is_none_or(|(_, best_key)| key < best_key) {
                best = Some((position, key));
            }
        }
        let (position, _) = best.expect("non-empty ready list");
        let node_index = ready.swap_remove(position);
        sim.apply(&nodes[node_index], info, budget);
        order.push(node_index);
        for &succ in &nodes[node_index].successors {
            nodes[succ].remaining_predecessors -= 1;
            if nodes[succ].remaining_predecessors == 0 {
                ready.push(succ);
            }
        }
    }
    if order.len() != nodes.len() {
        return None;
    }
    Some((order, sim.metrics))
}

fn strictly_better(before: Metrics, after: Metrics) -> bool {
    let improves = (0..CLASSES).any(|class| {
        after.0[class].excess_area < before.0[class].excess_area
            || after.0[class].excess_peak < before.0[class].excess_peak
    });
    improves
        && (0..CLASSES).all(|class| {
            let a = after.0[class];
            let b = before.0[class];
            a.live_area <= b.live_area
                && a.live_peak <= b.live_peak
                && a.excess_area <= b.excess_area
                && a.excess_peak <= b.excess_peak
        })
}

fn schedule_function(
    func: &mut IrFunction,
    target: Target,
    budget: PressureBudget,
    stats: &mut ShrinkStats,
) {
    if func.pass_policy.fp_environment_observable || func.blocks.is_empty() {
        return;
    }
    let Some((total_uses, definitions, info)) = collect_function_facts(func, target) else {
        return;
    };
    for (bi, block) in func.blocks.iter_mut().enumerate() {
        let n = block.instructions.len();
        if n < 3 {
            continue;
        }
        let mut remaining_original = total_uses.clone();
        let mut final_order: Vec<usize> = (0..n).collect();
        let mut start = 0usize;
        while start < n {
            if !schedulable(&block.instructions[start]) {
                // Phi operands are edge uses, not reads at the phi's lexical
                // position; never subtract them from this block's use counts.
                if !matches!(block.instructions[start], Instruction::Phi { .. }) {
                    for v in unique_instruction_uses(&block.instructions[start]) {
                        remaining_original[v as usize] =
                            remaining_original[v as usize].saturating_sub(1);
                    }
                }
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < n && schedulable(&block.instructions[end]) {
                end += 1;
            }
            if end - start >= 3 {
                if let Some((mut nodes, relevant)) =
                    build_nodes(&block.instructions, start, end, &info)
                {
                    stats.regions += 1;
                    let locally_defined: FxHashSet<u32> =
                        nodes.iter().filter_map(|node| node.dest).collect();
                    let before = simulate_original(
                        &nodes,
                        &remaining_original,
                        &relevant,
                        &locally_defined,
                        &info,
                        budget,
                    );
                    let before_excess: u64 = before
                        .0
                        .iter()
                        .map(|m| m.excess_area)
                        .fold(0u64, u64::saturating_add);
                    if before_excess != 0 {
                        stats.high_pressure_regions += 1;
                        stats.excess_area_before =
                            stats.excess_area_before.saturating_add(before_excess);
                        if let Some((order, after)) = candidate_order(
                            &mut nodes,
                            &remaining_original,
                            &relevant,
                            &locally_defined,
                            &info,
                            budget,
                        ) {
                            if strictly_better(before, after) {
                                for (offset, node_index) in order.into_iter().enumerate() {
                                    final_order[start + offset] = start + node_index;
                                }
                                let after_excess: u64 = after
                                    .0
                                    .iter()
                                    .map(|m| m.excess_area)
                                    .fold(0u64, u64::saturating_add);
                                stats.excess_area_after =
                                    stats.excess_area_after.saturating_add(after_excess);
                                stats.changed_regions += 1;
                            } else {
                                stats.excess_area_after =
                                    stats.excess_area_after.saturating_add(before_excess);
                            }
                        } else {
                            stats.excess_area_after =
                                stats.excess_area_after.saturating_add(before_excess);
                        }
                    }
                }
            }
            // The region's total use counts are order independent.  Update
            // from the original sequence so a rejected candidate and an
            // accepted candidate have identical facts at the next boundary.
            for inst in &block.instructions[start..end] {
                for v in unique_instruction_uses(inst) {
                    remaining_original[v as usize] =
                        remaining_original[v as usize].saturating_sub(1);
                }
            }
            start = end;
        }
        if final_order.iter().copied().eq(0..n) {
            continue;
        }
        // Cross-check that scheduling did not turn a same-block SSA use into
        // a use-before-def.  Phi operands are edge uses and are exempt.  The
        // DAG is authoritative; this assertion makes a future edit fail
        // loudly in a debug/test build rather than silently reaching codegen.
        let mut new_positions = vec![0usize; n];
        for (new, &old) in final_order.iter().enumerate() {
            new_positions[old] = new;
        }
        for (new_position, &old_position) in final_order.iter().enumerate() {
            let inst = &block.instructions[old_position];
            if matches!(inst, Instruction::Phi { .. }) {
                continue;
            }
            inst.for_each_used_value(|v| {
                if let Some((def_block, def_inst)) = definitions[v as usize]
                    && def_block == bi
                {
                    debug_assert!(new_positions[def_inst] < new_position);
                }
            });
        }
        let spans_valid = block.source_spans.len() == n;
        let mut old_instructions: Vec<Option<Instruction>> =
            std::mem::take(&mut block.instructions)
                .into_iter()
                .map(Some)
                .collect();
        block.instructions = final_order
            .iter()
            .map(|&old| {
                old_instructions[old]
                    .take()
                    .expect("each source instruction moved once")
            })
            .collect();
        if spans_valid {
            let old_spans = std::mem::take(&mut block.source_spans);
            block.source_spans = final_order.iter().map(|&old| old_spans[old]).collect();
        }
    }
}

pub(crate) fn run_with_budget(
    module: &mut IrModule,
    target: Target,
    budget: PressureBudget,
) -> ShrinkStats {
    let mut stats = ShrinkStats::default();
    for func in &mut module.functions {
        if !func.is_declaration {
            schedule_function(func, target, budget, &mut stats);
        }
    }
    if std::env::var_os("CCC_LR_SHRINK_STATS").is_some() {
        eprintln!(
            "[LR-SHRINK] regions={} high-pressure={} changed={} excess-area={}->{}",
            stats.regions,
            stats.high_pressure_regions,
            stats.changed_regions,
            stats.excess_area_before,
            stats.excess_area_after
        );
    }
    stats
}

pub(crate) fn run(module: &mut IrModule, target: Target) -> usize {
    run_with_budget(module, target, PressureBudget::for_target(target)).changed_regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, Terminator};

    fn copy(dest: u32, value: i32) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Const(IrConst::I32(value)),
        }
    }

    fn add(dest: u32, lhs: u32, rhs: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Value(Value(rhs)),
            ty: IrType::I32,
        }
    }

    fn function(instructions: Vec<Instruction>, result: u32) -> IrFunction {
        let mut f = IrFunction::new("pressure".into(), IrType::I32, vec![], false);
        f.next_value_id = 16;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: (0..instructions.len())
                .map(|i| Span::new(i as u32, i as u32 + 1, 0))
                .collect(),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(Value(result)))),
        });
        f
    }

    fn module(f: IrFunction) -> IrModule {
        let mut m = IrModule::new();
        m.functions.push(f);
        m
    }

    fn dests(f: &IrFunction) -> Vec<u32> {
        f.blocks[0]
            .instructions
            .iter()
            .filter_map(|i| i.dest().map(|d| d.0))
            .collect()
    }

    #[test]
    fn promotes_a_last_use_and_permuting_debug_spans() {
        let f = function(
            vec![
                copy(0, 1),
                copy(1, 2),
                add(2, 0, 0),
                copy(3, 3),
                add(4, 1, 3),
                add(5, 2, 4),
            ],
            5,
        );
        let mut m = module(f);
        let stats = run_with_budget(&mut m, Target::X86_64, PressureBudget { gp: 1, vector: 1 });
        assert_eq!(stats.changed_regions, 1);
        assert!(stats.excess_area_after < stats.excess_area_before);
        assert_eq!(dests(&m.functions[0]), vec![0, 2, 1, 3, 4, 5]);
        let starts: Vec<u32> = m.functions[0].blocks[0]
            .source_spans
            .iter()
            .map(|s| s.start)
            .collect();
        assert_eq!(starts, vec![0, 2, 1, 3, 4, 5]);
    }

    #[test]
    fn below_threshold_preserves_source_order() {
        let f = function(
            vec![
                copy(0, 1),
                copy(1, 2),
                add(2, 0, 1),
                copy(3, 4),
                add(4, 2, 3),
            ],
            4,
        );
        let before = dests(&f);
        let mut m = module(f);
        let stats = run_with_budget(
            &mut m,
            Target::X86_64,
            PressureBudget::for_target(Target::X86_64),
        );
        assert_eq!(stats.high_pressure_regions, 0);
        assert_eq!(dests(&m.functions[0]), before);
    }

    #[test]
    fn memory_is_a_hard_region_boundary() {
        let mut f = function(
            vec![
                copy(0, 1),
                copy(1, 2),
                add(2, 0, 1),
                Instruction::Store {
                    val: Operand::Value(Value(2)),
                    ptr: Value(15),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                copy(3, 3),
                copy(4, 4),
                add(5, 3, 4),
            ],
            5,
        );
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(5))));
        let mut m = module(f);
        let _ = run_with_budget(&mut m, Target::X86_64, PressureBudget { gp: 1, vector: 1 });
        let store = m.functions[0].blocks[0]
            .instructions
            .iter()
            .position(|i| matches!(i, Instruction::Store { .. }))
            .unwrap();
        assert_eq!(store, 3);
        assert_eq!(dests(&m.functions[0]), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn observable_fp_environment_disables_scheduling() {
        let mut f = function(
            vec![
                copy(0, 1),
                copy(1, 2),
                add(2, 0, 0),
                copy(3, 3),
                add(4, 1, 3),
                add(5, 2, 4),
            ],
            5,
        );
        f.pass_policy.fp_environment_observable = true;
        let before = dests(&f);
        let mut m = module(f);
        let stats = run_with_budget(&mut m, Target::X86_64, PressureBudget { gp: 1, vector: 1 });
        assert_eq!(stats.changed_regions, 0);
        assert_eq!(dests(&m.functions[0]), before);
    }

    #[test]
    fn register_weights_are_target_and_class_aware() {
        assert_eq!(value_weight(IrType::I64, Target::I686).weight, 2);
        assert_eq!(value_weight(IrType::I64, Target::X86_64).weight, 1);
        assert_eq!(value_weight(IrType::F64, Target::X86_64).class, VECTOR);
    }
}
