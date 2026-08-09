use crate::ir::reexports::{BlockId, IrFunction, Terminator};
use crate::pgo::ProfileData;
fn count(f: &IrFunction, l: BlockId, _p: &ProfileData) -> u64 {
    crate::pgo::active_profile_for_function(f)
        .map(|fp| fp.block_count(l))
        .unwrap_or(0)
}
pub fn cond_branch_prob(f: &IrFunction, t: BlockId, x: BlockId, p: &ProfileData) -> (f64, f64) {
    let a = count(f, t, p) as f64;
    let b = count(f, x, p) as f64;
    if a + b == 0.0 {
        (0.5, 0.5)
    } else {
        let q = a / (a + b);
        (q, 1.0 - q)
    }
}
pub fn should_layout_true_fallthrough(
    f: &IrFunction,
    t: BlockId,
    x: BlockId,
    p: &ProfileData,
) -> bool {
    cond_branch_prob(f, t, x, p).0 > 0.55
}
pub fn sorted_successors_by_hotness(
    f: &IrFunction,
    s: &[BlockId],
    p: &ProfileData,
) -> Vec<BlockId> {
    let mut v = s.iter().map(|&l| (l, count(f, l, p))).collect::<Vec<_>>();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    v.into_iter().map(|x| x.0).collect()
}
pub fn successors(t: &Terminator) -> Vec<BlockId> {
    match t {
        Terminator::Branch(x) => vec![*x],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![*true_label, *false_label],
        Terminator::Switch { cases, default, .. } => cases
            .iter()
            .map(|x| x.1)
            .chain(std::iter::once(*default))
            .collect(),
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.clone(),
        _ => vec![],
    }
}
