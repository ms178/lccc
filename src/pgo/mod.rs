//! PGO v4: fail-closed post-optimization profiling with safe profile merging.
pub(crate) mod branch_prob;
pub(crate) mod inline_pgo;
pub(crate) mod instrument;
pub(crate) mod layout;
pub(crate) mod profile;
pub(crate) mod unroll_pgo;
use crate::ir::reexports::{BlockId, IrFunction, IrModule};
use std::cell::{Cell, RefCell};
use crate::common::fx_hash::FxHashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default)]
pub struct FunctionProfile {
    pub total_count: u64,
    pub block_counts: FxHashMap<u32, u64>,
    /// Instrumented edge counts keyed by (src_label, dst_label). The virtual
    /// entry edge uses `crate::pgo::instrument::VENTRY` as src. Tree edges
    /// are absent; their counts are derived by flow conservation at profile
    /// use (see `profile::derive_block_counts`).
    pub edge_counts: FxHashMap<(u32, u32), u64>,
    pub cfg_hash: u64,
    pub entry_label: u32,
}
impl FunctionProfile {
    pub fn block_count(&self, l: BlockId) -> u64 {
        self.block_counts.get(&l.0).copied().unwrap_or(0)
    }
    pub fn edge_count(&self, src: u32, dst: u32) -> u64 {
        self.edge_counts.get(&(src, dst)).copied().unwrap_or(0)
    }
}
#[derive(Debug, Clone, Default)]
pub struct ProfileData {
    pub functions: FxHashMap<String, FunctionProfile>,
}
impl ProfileData {
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
    pub fn get(&self, n: &str) -> Option<&FunctionProfile> {
        self.functions.get(n).or_else(|| {
            self.functions
                .iter()
                .filter(|(k, _)| k.ends_with(&format!("::{}", n)))
                .map(|(_, f)| f)
                .max_by_key(|f| f.total_count)
        })
    }
    pub fn get_for_unit(&self, u: &str, n: &str) -> Option<&FunctionProfile> {
        profile::get_for_unit(self, u, n)
    }
    pub fn is_function_hot(&self, n: &str) -> bool {
        let Some(f) = self.get(n) else { return false };
        let m = self
            .functions
            .values()
            .map(|x| x.total_count)
            .max()
            .unwrap_or(0);
        m > 0 && f.total_count > 0 && f.total_count.saturating_mul(100) >= m
    }
    pub fn is_function_cold(&self, n: &str) -> bool {
        let Some(f) = self.get(n) else { return false };
        let m = self
            .functions
            .values()
            .map(|x| x.total_count)
            .max()
            .unwrap_or(0);
        m > 0 && f.total_count.saturating_mul(10000) < m
    }
    pub fn relative_frequency(&self, n: &str) -> f64 {
        let m = self
            .functions
            .values()
            .map(|x| x.total_count)
            .max()
            .unwrap_or(0);
        if m == 0 {
            return 0.0;
        }
        let f = if let Some(u) = ACTIVE_UNIT2.with(|slot| slot.borrow().clone()) {
            profile::get_for_unit(self, &u, n)
        } else {
            self.get(n)
        };
        f.map(|f| f.total_count as f64 / m as f64).unwrap_or(0.0)
    }
    pub fn max_total_for_unit(&self, u: &str) -> u64 {
        let p = format!("{:016x}::", profile::unit_hash(u));
        self.functions
            .iter()
            .filter(|(k, _)| k.starts_with(&p))
            .map(|(_, f)| f.total_count)
            .max()
            .unwrap_or(0)
    }
}
thread_local! {
    /// The currently compiled translation unit. LCCC may compile more than one
    /// input in a process, so this cannot be a process-wide OnceLock.
    static ACTIVE_UNIT: RefCell<Option<String>> = const { RefCell::new(None) };
    static ACTIVE_PROFILE_VALID: Cell<bool> = const { Cell::new(false) };
}
static ACTIVE: OnceLock<ProfileData> = OnceLock::new();
thread_local! {
    /// Derived (edge-solved) per-function profiles for the current unit.
    /// Box::leak'd once per unit (replaced at the next propagate_profile);
    /// consumers only run between fills, so the pointer stays valid.
    static DERIVED_PTR: Cell<*const FxHashMap<String, FunctionProfile>> =
        const { Cell::new(std::ptr::null()) };
    static ACTIVE_UNIT2: RefCell<Option<String>> = const { RefCell::new(None) };
    static PREPASS_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// Activate the profile for PRE-PASS consumers (PGO-guided inlining, loop
/// unrolling). Called before `run_passes`; name+unit-keyed entry counts are
/// stable across gen/use even when the optimizer changes the CFG.
pub fn prepass_activate(unit: &str) {
    if get_pgo_profile().is_none() {
        return;
    }
    ACTIVE_UNIT2.with(|slot| *slot.borrow_mut() = Some(unit.to_string()));
    PREPASS_ACTIVE.with(|a| a.set(true));
}

pub fn prepass_is_active() -> bool {
    PREPASS_ACTIVE.with(Cell::get)
}

/// Name-keyed, unit-scoped total count for pre-pass consumers.
pub fn total_count_for(name: &str) -> u64 {
    let Some(p) = get_pgo_profile() else { return 0 };
    if let Some(u) = ACTIVE_UNIT2.with(|slot| slot.borrow().clone()) {
        profile::get_for_unit(p, &u, name)
            .map(|f| f.total_count)
            .unwrap_or(0)
    } else {
        p.get(name).map(|f| f.total_count).unwrap_or(0)
    }
}
pub fn init_pgo_profile(path: Option<&str>) {
    let d = match path {
        Some(p) => match profile::load_profile(p) {
            Ok(x) => {
                eprintln!(
                    "lccc: PGO v3: loaded {} functions from {}",
                    x.functions.len(),
                    p
                );
                x
            }
            Err(e) => {
                eprintln!("lccc: PGO warning: {} -- continuing without profile", e);
                ProfileData::default()
            }
        },
        None => ProfileData::default(),
    };
    let _ = ACTIVE.set(d);
}
pub fn get_pgo_profile() -> Option<&'static ProfileData> {
    ACTIVE.get().filter(|x| !x.is_empty())
}
/// Validate only; no guessed counts are propagated into unmeasured branches.
pub fn propagate_profile(m: &mut IrModule, u: &str) {
    let Some(p) = get_pgo_profile() else { return };
    ACTIVE_UNIT.with(|slot| *slot.borrow_mut() = Some(u.to_string()));
    ACTIVE_PROFILE_VALID.with(|valid| valid.set(false));
    let mut bad = 0;
    let mut good = 0;
    for f in &m.functions {
        if f.is_declaration || f.blocks.is_empty() {
            continue;
        }
        let h = profile::cfg_fingerprint(&f.name, u, f);
        if profile::get_for_unit_cfg(p, u, &f.name, h).is_none() {
            bad += 1
        } else {
            good += 1
        }
    }
    ACTIVE_PROFILE_VALID.with(|valid| valid.set(good > 0));
    if bad > 0 {
        eprintln!(
            "lccc: PGO v4: ignored {} stale/mismatched function profiles",
            bad
        )
    }
    // Derive block counts and tree-edge counts from the instrumented edge
    // counts (flow conservation) into a leaked per-unit map that
    // active_profile_for_function prefers.
    let mut derived: FxHashMap<String, FunctionProfile> = FxHashMap::default();
    for f in &m.functions {
        if f.is_declaration || f.blocks.is_empty() {
            continue;
        }
        let h = profile::cfg_fingerprint(&f.name, u, f);
        let Some(fp) = profile::get_for_unit_cfg(p, u, &f.name, h) else {
            continue;
        };
        let mut copy = fp.clone();
        profile::derive_block_counts(f, &mut copy);
        derived.insert(profile::function_key(u, &f.name), copy);
    }
    if !derived.is_empty() {
        let leaked: &'static FxHashMap<String, FunctionProfile> = Box::leak(Box::new(derived));
        DERIVED_PTR.with(|d| d.set(leaked as *const _));
    }
}

/// Return the exact, CFG-validated profile for a function in the translation
/// unit currently being compiled. Name-only lookup is intentionally avoided:
/// static functions with the same spelling in different TUs are common.
pub fn active_profile_for_function(f: &IrFunction) -> Option<&'static FunctionProfile> {
    ACTIVE_UNIT.with(|slot| {
        let unit = slot.borrow();
        let unit = unit.as_deref()?;
        let key = profile::function_key(unit, &f.name);
        let derived = DERIVED_PTR.with(|d| {
            let p = d.get();
            if p.is_null() {
                None
            } else {
                // SAFETY: leaked once per unit before codegen; only replaced
                // at the next propagate_profile call, never concurrently.
                unsafe { (*p).get(&key) }
            }
        });
        if let Some(fp) = derived {
            return Some(fp);
        }
        let p = get_pgo_profile()?;
        profile_for_function(p, unit, f)
    })
}

pub fn has_active_valid_profile() -> bool {
    ACTIVE_PROFILE_VALID.with(Cell::get)
}
pub fn profile_for_function<'a>(
    p: &'a ProfileData,
    u: &str,
    f: &IrFunction,
) -> Option<&'a FunctionProfile> {
    let h = profile::cfg_fingerprint(&f.name, u, f);
    profile::get_for_unit_cfg(p, u, &f.name, h)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entry_hotness_not_cfg_size() {
        let mut p = ProfileData::default();
        p.functions.insert(
            "a::x".into(),
            FunctionProfile {
                total_count: 10,
                block_counts: FxHashMap::default(),
                edge_counts: FxHashMap::default(),
                cfg_hash: 1,
                entry_label: 0,
            },
        );
        p.functions.insert(
            "b::y".into(),
            FunctionProfile {
                total_count: 9,
                block_counts: FxHashMap::default(),
                edge_counts: FxHashMap::default(),
                cfg_hash: 2,
                entry_label: 0,
            },
        );
        assert!(p.is_function_hot("x"));
    }
}
