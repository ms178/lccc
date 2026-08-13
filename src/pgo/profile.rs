//! PGO v4 profile identity, deterministic merge, and fail-closed loading.
use super::{FunctionProfile, ProfileData};
use crate::ir::reexports::{IrFunction, Terminator};
use crate::common::fx_hash::FxHashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const FNV0: u64 = 0xcbf29ce484222325;
const FNV1: u64 = 0x100000001b3;
fn hb(h: &mut u64, b: u8) {
    *h = (*h ^ b as u64).wrapping_mul(FNV1);
}
fn hu(h: &mut u64, n: u64) {
    for b in n.to_le_bytes() {
        hb(h, b);
    }
}

/// Stable across build-directory relocation: basename plus source-content hash.
pub fn unit_identity(path: &str) -> String {
    if path == "-" {
        return "stdin".into();
    }
    let base = Path::new(path)
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or(path);
    let bytes = fs::read(path).unwrap_or_else(|_| path.as_bytes().to_vec());
    let mut h = FNV0;
    for b in bytes {
        hb(&mut h, b);
    }
    format!("{}#{:016x}", base, h)
}
pub fn unit_hash(unit: &str) -> u64 {
    let mut h = FNV0;
    for b in unit.as_bytes() {
        hb(&mut h, *b);
    }
    h
}
pub fn function_key(unit: &str, name: &str) -> String {
    format!("{:016x}::{}", unit_hash(unit), name)
}

/// Post-optimization CFG fingerprint. Profiles with a changed module are ignored.
pub fn cfg_fingerprint(name: &str, unit: &str, f: &IrFunction) -> u64 {
    let mut h = FNV0;
    for b in name.as_bytes() {
        hb(&mut h, *b);
    }
    hu(&mut h, unit_hash(unit));
    hu(&mut h, f.blocks.len() as u64);
    for b in &f.blocks {
        hu(&mut h, b.label.0 as u64);
        hu(&mut h, b.instructions.len() as u64);
        match &b.terminator {
            Terminator::Branch(x) => {
                hb(&mut h, 1);
                hu(&mut h, x.0 as u64);
            }
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                hb(&mut h, 2);
                hu(&mut h, true_label.0 as u64);
                hu(&mut h, false_label.0 as u64);
            }
            Terminator::Switch { cases, default, .. } => {
                hb(&mut h, 3);
                hu(&mut h, cases.len() as u64);
                for (v, x) in cases {
                    hu(&mut h, *v as u64);
                    hu(&mut h, x.0 as u64);
                }
                hu(&mut h, default.0 as u64);
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => {
                hb(&mut h, 4);
                for x in possible_targets {
                    hu(&mut h, x.0 as u64);
                }
            }
            Terminator::Return(_) => hb(&mut h, 5),
            Terminator::Unreachable => hb(&mut h, 6),
        }
    }
    h
}

pub fn resolve_output_path(path: &str, unit: &str) -> PathBuf {
    let p = if path.is_empty() {
        Path::new(".")
    } else {
        Path::new(path)
    };
    if p.extension().and_then(|x| x.to_str()) == Some("profraw") {
        if let Some(q) = p.parent() {
            let _ = fs::create_dir_all(q);
        }
        return p.into();
    }
    let _ = fs::create_dir_all(p);
    p.join(format!(
        "lccc-{:016x}-{}-{}.profraw",
        unit_hash(unit),
        sanitize(unit),
        std::process::id()
    ))
}
fn sanitize(s: &str) -> String {
    let x: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if x.is_empty() {
        "unit".into()
    } else {
        x
    }
}

pub fn load_profile(path: &str) -> std::io::Result<ProfileData> {
    let p = Path::new(path);
    let mut d = ProfileData::default();
    let mut files = Vec::new();
    if p.is_dir() {
        for e in fs::read_dir(p)? {
            let q = e?.path();
            if q.is_file()
                && q.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("lccc-") && n.ends_with(".profraw"))
                    .unwrap_or(false)
            {
                files.push(q)
            }
        }
        files.sort();
        if files.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no lccc profile files",
            ));
        }
    } else {
        files.push(p.into());
    }
    for q in files {
        parse_file(&q, &mut d)?;
    }
    if d.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "empty profile",
        ));
    }
    Ok(d)
}
fn bad(p: &Path, n: usize, s: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{}:{}: {}", p.display(), n, s),
    )
}
fn parse_file(p: &Path, d: &mut ProfileData) -> std::io::Result<()> {
    let mut name = None;
    let mut cur = None;
    let mut hash: u64 = 0;
    let mut entry: u32 = 0;
    let mut unit: u64 = 0;
    let mut flush =
        |name: &mut Option<String>, cur: &mut Option<FunctionProfile>| -> std::io::Result<()> {
            if let (Some(n), Some(mut f)) = (name.take(), cur.take()) {
                f.total_count = f
                    .edge_counts
                    .get(&(crate::pgo::instrument::VENTRY, f.entry_label))
                    .copied()
                    .unwrap_or_else(|| {
                        f.edge_counts.values().copied().max().unwrap_or(0)
                    });
                d.merge(n, f)?;
            }
            Ok(())
        };
    for (i, line) in BufReader::new(fs::File::open(p)?).lines().enumerate() {
        let n = i + 1;
        let l = line?;
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let mut a = l.split_whitespace();
        match a.next().unwrap_or("") {
            "lccc-pgo-v3" | "lccc-pgo-v4" => {
                hash = a
                    .next()
                    .ok_or_else(|| bad(p, n, "missing hash"))?
                    .parse()
                    .map_err(|_| bad(p, n, "bad hash"))?;
                entry = a
                    .next()
                    .ok_or_else(|| bad(p, n, "missing entry"))?
                    .parse()
                    .map_err(|_| bad(p, n, "bad entry"))?;
                unit = a
                    .next()
                    .ok_or_else(|| bad(p, n, "missing unit"))?
                    .parse()
                    .map_err(|_| bad(p, n, "bad unit"))?;
            }
            "func" => {
                flush(&mut name, &mut cur)?;
                let x = a.collect::<Vec<_>>().join(" ");
                if x.is_empty() || !x.starts_with(&format!("{:016x}::", unit)) {
                    return Err(bad(p, n, "bad function key"));
                }
                name = Some(x);
                cur = Some(FunctionProfile {
                    total_count: 0,
                    block_counts: FxHashMap::default(),
                    edge_counts: FxHashMap::default(),
                    cfg_hash: hash,
                    entry_label: entry,
                });
            }
            "f" => {
                let f = cur
                    .as_mut()
                    .ok_or_else(|| bad(p, n, "entry count before func"))?;
                let c = a.next().ok_or_else(|| bad(p, n, "missing count"))?;
                let c: u64 = c.parse().map_err(|_| bad(p, n, "bad count"))?;
                *f.edge_counts
                    .entry((crate::pgo::instrument::VENTRY, f.entry_label))
                    .or_insert(0) = c;
            }
            "e" => {
                let f = cur
                    .as_mut()
                    .ok_or_else(|| bad(p, n, "edge before func"))?;
                let src: u32 = a
                    .next()
                    .ok_or_else(|| bad(p, n, "missing edge src"))?
                    .parse()
                    .map_err(|_| bad(p, n, "bad edge src"))?;
                let dst: u32 = a
                    .next()
                    .ok_or_else(|| bad(p, n, "missing edge dst"))?
                    .parse()
                    .map_err(|_| bad(p, n, "bad edge dst"))?;
                let c = a.next().ok_or_else(|| bad(p, n, "missing count"))?;
                let c: u64 = c.parse().map_err(|_| bad(p, n, "bad count"))?;
                *f.edge_counts.entry((src, dst)).or_insert(0) = f
                    .edge_counts
                    .get(&(src, dst))
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(c);
            }
            x => {
                // v3 block-count lines accepted for compatibility.
                let f = cur
                    .as_mut()
                    .ok_or_else(|| bad(p, n, "counter before func"))?;
                let c = a.next().ok_or_else(|| bad(p, n, "missing count"))?;
                let b: u32 = x.parse().map_err(|_| bad(p, n, "bad block"))?;
                let c: u64 = c.parse().map_err(|_| bad(p, n, "bad count"))?;
                *f.block_counts.entry(b).or_insert(0) = f
                    .block_counts
                    .get(&b)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(c);
            }
        }
    }
    flush(&mut name, &mut cur)
}
impl ProfileData {
    fn merge(&mut self, n: String, f: FunctionProfile) -> std::io::Result<()> {
        let k = if let Some(x) = self.functions.get(&n) {
            if x.cfg_hash != f.cfg_hash {
                format!("{}#cfg{:016x}", n, f.cfg_hash)
            } else {
                n.clone()
            }
        } else {
            n
        };
        if let Some(x) = self.functions.get_mut(&k) {
            for (b, c) in f.block_counts {
                *x.block_counts.entry(b).or_insert(0) = x
                    .block_counts
                    .get(&b)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(c);
            }
            for (e, c) in f.edge_counts {
                *x.edge_counts.entry(e).or_insert(0) = x
                    .edge_counts
                    .get(&e)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(c);
            }
            x.total_count = x
                .edge_counts
                .get(&(crate::pgo::instrument::VENTRY, x.entry_label))
                .copied()
                .unwrap_or(0);
        } else {
            self.functions.insert(k, f);
        }
        Ok(())
    }
}
pub fn get_for_unit<'a>(d: &'a ProfileData, u: &str, n: &str) -> Option<&'a FunctionProfile> {
    let k = function_key(u, n);
    if let Some(x) = d.functions.get(&k) {
        return Some(x);
    }
    d.functions
        .iter()
        .filter(|(x, _)| x.starts_with(&format!("{}#", k)))
        .map(|(_, x)| x)
        .max_by_key(|x| x.total_count)
}
pub fn get_for_unit_cfg<'a>(
    d: &'a ProfileData,
    u: &str,
    n: &str,
    h: u64,
) -> Option<&'a FunctionProfile> {
    let k = function_key(u, n);
    d.functions
        .iter()
        .filter(|(x, f)| (x.as_str() == k || x.starts_with(&format!("{}#", k))) && f.cfg_hash == h)
        .map(|(_, f)| f)
        .next()
}
pub fn write_text_profile(p: &Path, d: &ProfileData) -> std::io::Result<()> {
    if let Some(q) = p.parent() {
        fs::create_dir_all(q)?
    }
    let mut f = fs::File::create(p)?;
    for (n, x) in &d.functions {
        writeln!(
            f,
            "lccc-pgo-v3 {} {} {}",
            x.cfg_hash,
            x.entry_label,
            n.split("::").next().unwrap_or("0")
        )?;
        writeln!(f, "func {}", n)?;
        for (b, c) in &x.block_counts {
            writeln!(f, "{} {}", b, c)?;
        }
    }
    Ok(())
}

/// Derive every block count and every tree-edge count from the instrumented
/// edge counts by flow conservation (Knuth–Stevenson / GCC gcov / LLVM).
pub fn derive_block_counts(f: &IrFunction, fp: &mut super::FunctionProfile) {
    use crate::common::fx_hash::FxHashMap;
    use crate::ir::reexports::Terminator;
    use crate::pgo::instrument::VENTRY;
    if fp.edge_counts.is_empty() {
        return; // v3 block-count profile: nothing to derive
    }
    fn succs(t: &Terminator) -> Vec<u32> {
        match t {
            Terminator::Branch(x) => vec![x.0],
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => vec![true_label.0, false_label.0],
            Terminator::Switch { cases, default, .. } => {
                let mut v: Vec<u32> = cases.iter().map(|(_, b)| b.0).collect();
                v.push(default.0);
                v
            }
            Terminator::IndirectBranch {
                possible_targets, ..
            } => possible_targets.iter().map(|b| b.0).collect(),
            _ => vec![],
        }
    }
    let mut known_in: FxHashMap<u32, u64> = FxHashMap::default();
    let mut known_out: FxHashMap<u32, u64> = FxHashMap::default();
    let mut unknown_in: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    let mut unknown_out: FxHashMap<u32, Vec<(u32, u32)>> = FxHashMap::default();
    for b in &f.blocks {
        for d in succs(&b.terminator) {
            let e = (b.label.0, d);
            match fp.edge_counts.get(&e) {
                Some(&c) => {
                    *known_in.entry(d).or_insert(0) += c;
                    *known_out.entry(b.label.0).or_insert(0) += c;
                }
                None => {
                    unknown_in.entry(d).or_default().push(e);
                    unknown_out.entry(b.label.0).or_default().push(e);
                }
            }
        }
    }
    let entry = f.blocks.first().map(|b| b.label.0).unwrap_or(0);
    let entry_count = fp.edge_counts.get(&(VENTRY, entry)).copied().unwrap_or(0);

    // Children-before-parents order (reverse BFS over unknown edges).
    let mut order = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(entry);
        seen.insert(entry);
        while let Some(b) = queue.pop_front() {
            order.push(b);
            if let Some(outs) = unknown_out.get(&b) {
                for &(_, d) in outs {
                    if seen.insert(d) {
                        queue.push_back(d);
                    }
                }
            }
        }
    }
    let mut out_sum: FxHashMap<u32, u64> = known_out.clone();
    let mut in_sum: FxHashMap<u32, u64> = known_in.clone();
    let mut counts: FxHashMap<u32, u64> = FxHashMap::default();
    let mut derived_edges: Vec<((u32, u32), u64)> = Vec::new();
    for &b in order.iter().rev() {
        if b == entry {
            counts.insert(b, entry_count);
            continue;
        }
        let c = out_sum.get(&b).copied().unwrap_or(0);
        counts.insert(b, c);
        if let Some(ins) = unknown_in.get(&b) {
            if let Some(&pe) = ins.first() {
                let pc = c.saturating_sub(in_sum.get(&b).copied().unwrap_or(0));
                derived_edges.push((pe, pc));
                *out_sum.entry(pe.0).or_insert(0) =
                    out_sum.get(&pe.0).copied().unwrap_or(0).saturating_add(pc);
            }
        }
    }
    for (b, c) in counts {
        fp.block_counts.insert(b, c);
    }
    for (e, c) in derived_edges {
        let v = fp.edge_counts.entry(e).or_insert(0);
        *v = v.saturating_add(c);
    }
    fp.total_count = entry_count;
}
