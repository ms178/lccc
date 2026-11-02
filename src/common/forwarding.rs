//! Resolve an immutable out-degree-one graph without recursion or depth caps.
//! Each forwarding vertex is expanded once. A terminal result includes the
//! last forwarding vertex: SSA PHI translation needs that edge, not the start.
//! Cycles and every prefix entering a cycle deliberately have no result.
use super::fx_hash::{FxHashMap, FxHashSet};
use std::hash::Hash;

pub(crate) fn resolve<K: Copy + Eq + Hash>(edges: &FxHashMap<K, K>) -> FxHashMap<K, (K, K)> {
    let mut resolved = FxHashMap::with_capacity_and_hasher(edges.len(), Default::default());
    let mut visited = FxHashSet::with_capacity_and_hasher(edges.len(), Default::default());
    let mut path = Vec::new();
    for &start in edges.keys() {
        if visited.contains(&start) {
            continue;
        }
        let mut current = start;
        let end = loop {
            if let Some(&end) = resolved.get(&current) {
                break Some(end);
            }
            let Some(&next) = edges.get(&current) else {
                break path.last().copied().map(|last| (current, last));
            };
            if !visited.insert(current) {
                break None;
            }
            path.push(current);
            current = next;
        };
        if let Some(end) = end {
            for node in path.drain(..) {
                resolved.insert(node, end);
            }
        } else {
            path.clear();
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustive_five_vertex_graphs_match_independent_walks() {
        // Absent edge, five vertices, or a common external terminal: 7^5.
        for encoding in 0..7u32.pow(5) {
            let mut code = encoding;
            let mut edges = FxHashMap::default();
            for node in 0..5 {
                let digit = code % 7;
                code /= 7;
                if digit != 0 {
                    edges.insert(node, digit - 1);
                }
            }
            let got = resolve(&edges);
            for &start in edges.keys() {
                let mut seen = std::collections::BTreeSet::new();
                let mut current = start;
                let mut last = start;
                let expected = loop {
                    if !seen.insert(current) {
                        break None;
                    }
                    if let Some(&next) = edges.get(&current) {
                        last = current;
                        current = next;
                    } else {
                        break Some((current, last));
                    }
                };
                assert_eq!(
                    got.get(&start).copied(),
                    expected,
                    "graph={encoding}, start={start}"
                );
            }
        }
    }

    #[test]
    fn long_sparse_chains_and_shared_cycle_tails_are_not_depth_limited() {
        let mut edges = FxHashMap::default();
        for i in 0..10_000u32 {
            edges.insert(i * 3, (i + 1) * 3);
        }
        edges.insert(u32::MAX, 0);
        let r = resolve(&edges);
        assert_eq!(r.len(), 10_001);
        assert_eq!(r[&u32::MAX], (30_000, 29_997));
        edges.insert(30_000, 15_000);
        assert!(resolve(&edges).is_empty());
    }
}
