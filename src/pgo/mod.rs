//! Profile-guided optimization: fail-closed, post-optimization profiling with safe
//! profile merging, and the consumers (inlining, unrolling, layout,
//! devirtualization) that a loaded profile drives.
pub(crate) mod branch_prob;
pub(crate) mod inline_pgo;
pub(crate) mod instrument;
pub(crate) mod layout;
pub(crate) mod profile;
pub(crate) mod promote;
pub(crate) mod summary;
pub(crate) mod unroll_pgo;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::{BlockId, IrFunction, IrModule};
use std::cell::{Cell, RefCell};
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
    /// PRE-pass structural fingerprint: the profile identity. Stable across
    /// generate/use builds even when profile-guided transforms (inlining,
    /// unrolling, devirtualization) change the post-pass CFG.
    pub cfg_hash: u64,
    /// POST-pass structural fingerprint: detects CFG drift from
    /// profile-guided transforms; edge/layout consumers degrade gracefully
    /// instead of dropping the whole function.
    pub post_hash: u64,
    pub entry_label: u32,
    /// Indirect-call value profiles: site ordinal -> top callees.
    pub value_sites: Vec<ValueSite>,
}

/// One indirect-call site's recorded callee distribution.
#[derive(Debug, Clone, Default)]
pub struct ValueSite {
    pub ordinal: usize,
    pub total: u64,
    pub sig: String,
    /// (callee name, count, linkage flags), sorted by count descending.
    /// flags bit0 = static (cross-TU direct calls to statics cannot link).
    pub targets: Vec<(String, u64, u64)>,
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

/// Pre-pass (mem2reg-time) fingerprints: the profile identity, stable across
/// gen/use. Filled by the driver right after mem2reg. Mutex-backed because
/// `FxHashMap::default()` is not const (thread_local! requires const inits);
/// compilation is single-threaded per TU so the lock is uncontended.
static PRE_HASHES: std::sync::LazyLock<std::sync::Mutex<FxHashMap<String, u64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));
/// Post-pass fingerprints: CFG-drift detection. Filled by the driver right
/// after run_passes (before promotion/instrumentation).
static POST_HASHES: std::sync::LazyLock<std::sync::Mutex<FxHashMap<String, u64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));
/// Promoted-hot block labels (unit, function) -> labels, from
/// promote::promote_indirect_calls. Labels are recorded BEFORE the label
/// renumber pass; `remap_promoted_hot` (called by the driver right after
/// renumbering, which knows the exact old->new map) translates them, so
/// layout sees post-renumber labels.
static PROMOTED_HOT: std::sync::LazyLock<
    std::sync::Mutex<FxHashMap<(String, String), FxHashSet<u32>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));

pub fn stash_pre_hashes(map: FxHashMap<String, u64>) {
    *PRE_HASHES.lock().unwrap() = map;
}
pub fn stash_post_hashes(map: FxHashMap<String, u64>) {
    *POST_HASHES.lock().unwrap() = map;
}
pub fn pre_hash_for(name: &str) -> u64 {
    PRE_HASHES.lock().unwrap().get(name).copied().unwrap_or(0)
}
pub fn post_hash_for(name: &str) -> u64 {
    POST_HASHES.lock().unwrap().get(name).copied().unwrap_or(0)
}
pub fn pre_hashes() -> FxHashMap<String, u64> {
    PRE_HASHES.lock().unwrap().clone()
}
pub fn post_hashes() -> FxHashMap<String, u64> {
    POST_HASHES.lock().unwrap().clone()
}
pub fn record_promoted_hot(u: &str, map: FxHashMap<String, FxHashSet<u32>>) {
    let mut m = PROMOTED_HOT.lock().unwrap();
    for (fname, labels) in map {
        m.insert((u.to_string(), fname), labels);
    }
}
pub fn promoted_hot_labels(u: &str, fname: &str) -> FxHashSet<u32> {
    PROMOTED_HOT
        .lock()
        .unwrap()
        .get(&(u.to_string(), fname.to_string()))
        .cloned()
        .unwrap_or_default()
}

/// Profile-driven switch lowering hint for one switch block.
/// `hot_case` = the case value/target that accounts for >= 50% of the
/// block's executions (hoist it out of the jump table — LLVM's
/// profile-guided switch partitioning); `force_chain` = the switch block is
/// COLD per the profile summary (a jump table would waste rodata + I-cache
/// on a path that barely runs — lower as a compare chain instead, even when
/// the case set is dense).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchHint {
    pub hot_case: Option<(i64, u32)>,
    pub force_chain: bool,
}

/// Pending hint consumed by the backend's `emit_switch`. Set by generation.rs
/// (which knows the current block label) immediately before the terminator is
/// emitted and taken by the backend default `emit_switch`. Codegen is
/// single-threaded and per-TU sequential, so a process-wide scratch is safe.
static PENDING_SWITCH_HINT: std::sync::LazyLock<std::sync::Mutex<Option<SwitchHint>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

pub fn set_switch_hint(h: Option<SwitchHint>) {
    *PENDING_SWITCH_HINT.lock().unwrap() = h;
}
pub fn take_switch_hint() -> Option<SwitchHint> {
    PENDING_SWITCH_HINT.lock().unwrap().take()
}

/// Per-unit switch hints keyed by block label (labels are TU-unique after
/// the renumber pass; layout runs immediately before codegen for the same
/// unit, so a fresh map per unit is correct even for multi-TU invocations).
static SWITCH_HINTS: std::sync::LazyLock<std::sync::Mutex<FxHashMap<u32, SwitchHint>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));

/// Replace the hint map (called once per unit by layout_module).
pub fn record_switch_hints(map: FxHashMap<u32, SwitchHint>) {
    *SWITCH_HINTS.lock().unwrap() = map;
}
pub fn switch_hint(label: u32) -> Option<SwitchHint> {
    SWITCH_HINTS.lock().unwrap().get(&label).copied()
}
pub fn switch_hints_snapshot() -> FxHashMap<u32, SwitchHint> {
    SWITCH_HINTS.lock().unwrap().clone()
}

/// Per-unit preferred-fallthrough successor for conditional branches, keyed
/// by the branch block's label after renumbering. Layout chooses the successor
/// with the greater edge count so the hot path falls through without reordering
/// blocks.
static COND_FALLTHROUGHS: std::sync::LazyLock<std::sync::Mutex<FxHashMap<u32, u32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));
/// Pending preferred-fallthrough for the conditional branch being emitted
/// (block label is set right before codegen of the terminator and taken by the
/// backend's emit_cond_branch_blocks_impl). Codegen is single-threaded and
/// per-TU sequential, so a process-wide scratch is safe.
static PENDING_COND_FALLTHROUGH: std::sync::LazyLock<std::sync::Mutex<Option<u32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

/// Replace the per-unit cond-branch fallthrough map (called once per unit by
/// layout_module).
pub fn record_cond_fallthroughs(map: FxHashMap<u32, u32>) {
    *COND_FALLTHROUGHS.lock().unwrap() = map;
}
/// The preferred fallthrough successor (block label) for a conditional branch
/// whose source block is `label`, if the profile was informative for it.
pub fn cond_fallthrough(label: u32) -> Option<u32> {
    COND_FALLTHROUGHS.lock().unwrap().get(&label).copied()
}
/// Set the pending preferred-fallthrough before codegen of a CondBranch.
pub fn set_cond_fallthrough(h: Option<u32>) {
    *PENDING_COND_FALLTHROUGH.lock().unwrap() = h;
}
/// Take (and clear) the pending preferred-fallthrough in the backend.
pub fn take_cond_fallthrough() -> Option<u32> {
    PENDING_COND_FALLTHROUGH.lock().unwrap().take()
}

/// Per-unit basic-block alignment hints, keyed by block label. Values are
/// alignment exponents: 4 means 16 bytes and 5 means 32 bytes. Layout derives
/// hints for hot loop headers and join points; codegen consumes them directly
/// before emitting each label.
static BLOCK_ALIGNS: std::sync::LazyLock<std::sync::Mutex<FxHashMap<u32, u8>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(FxHashMap::default()));

/// Replace the per-unit block-alignment map (called once per unit by layout).
pub fn record_block_aligns(map: FxHashMap<u32, u8>) {
    *BLOCK_ALIGNS.lock().unwrap() = map;
}
/// The alignment (log2) for `label`, if the profile made it a hot branch
/// target. Consumed by codegen immediately before the block label is emitted.
pub fn block_align(label: u32) -> Option<u8> {
    BLOCK_ALIGNS.lock().unwrap().get(&label).copied()
}
/// True when profile-driven block alignment is engaged (used by codegen to
/// skip the map lookups on plain builds).
pub fn block_align_active() -> bool {
    !BLOCK_ALIGNS.lock().unwrap().is_empty()
}

/// Translate recorded promoted-block labels through the label-renumber map
/// (old label -> new label; labels are TU-unique). Called by the driver right
/// after the renumber pass; labels whose block vanished keep their value (the
/// relocation then simply fails to find them and leaves the block in place —
/// harmless).
pub fn remap_promoted_hot(remap: &FxHashMap<u32, u32>) {
    let mut m = PROMOTED_HOT.lock().unwrap();
    for labels in m.values_mut() {
        let old: Vec<u32> = labels.iter().copied().collect();
        labels.clear();
        for l in old {
            labels.insert(remap.get(&l).copied().unwrap_or(l));
        }
    }
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

/// The unit currently being compiled (pre-pass unit preferred, post-pass
/// fallback) — used by summary-aware consumers.
pub fn active_unit() -> Option<String> {
    ACTIVE_UNIT2
        .with(|s| s.borrow().clone())
        .or_else(|| ACTIVE_UNIT.with(|s| s.borrow().clone()))
}

/// Raw instrumented-edge profile for a function in the pre-pass unit.
/// Callers needing block or tree-edge counts must derive them from the current
/// `IrFunction` with `profile::derive_block_counts`.
pub fn prepass_profile(name: &str) -> Option<FunctionProfile> {
    let p = get_pgo_profile()?;
    let u = ACTIVE_UNIT2.with(|slot| slot.borrow().clone())?;
    profile::get_for_unit(p, &u, name).cloned()
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
                    "lccc: PGO: loaded {} functions from {}",
                    x.functions.len(),
                    p
                );
                // Debug/inspection hook: dump the loaded profile as text for
                // a human-readable view (LCCC_PGO_DUMP_TEXT=<file>).
                if let Ok(txt) = std::env::var("LCCC_PGO_DUMP_TEXT") {
                    match profile::write_text_profile(std::path::Path::new(&txt), &x) {
                        Ok(()) => eprintln!("lccc: PGO: wrote text profile to {}", txt),
                        Err(e) => eprintln!("lccc: PGO warning: text dump failed: {}", e),
                    }
                }
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
    let mut drifted = 0;
    for f in &m.functions {
        if f.is_declaration || f.blocks.is_empty() {
            continue;
        }
        // Identity is the PRE-pass fingerprint (stable across gen/use);
        // the post-pass fingerprint detects CFG drift from PGO transforms.
        let h0 = pre_hash_for(&f.name);
        let Some(fp) = profile::get_for_unit_cfg(p, u, &f.name, h0) else {
            bad += 1;
            continue;
        };
        good += 1;
        if fp.post_hash != 0 && fp.post_hash != post_hash_for(&f.name) {
            drifted += 1;
        }
    }
    ACTIVE_PROFILE_VALID.with(|valid| valid.set(good > 0));
    if bad > 0 {
        eprintln!(
            "lccc: PGO: no profile for {} function(s) in this unit (new/changed code?)",
            bad
        )
    }
    if drifted > 0 {
        eprintln!(
            "lccc: PGO: {} function(s) drifted from the training CFG (PGO-guided transforms); edge profiles degrade gracefully",
            drifted
        )
    }
    // Derive block counts and tree-edge counts from the instrumented edge
    // counts (flow conservation) into a leaked per-unit map that
    // active_profile_for_function prefers.
    //
    // DRIFT GATE (red-team audit): a function whose POST-pass CFG no longer
    // matches the training build has been structurally changed by
    // profile-guided transforms (inlining/unrolling fired differently than in
    // the training build). Its edge counts are keyed by the TRAINING labels;
    // after block insertion/removal the surviving labels no longer denote the
    // same edges, so every derived block/backedge count is unreliable. We
    // therefore only publish edge-DERIVED data for functions whose post_hash
    // matches (or legacy profiles with post_hash == 0). Drifted functions
    // keep their name/unit-keyed `total_count` (stable, used for hot/cold
    // section classification) but get NO edge-derived block counts — edge
    // consumers (block layout, branch probability, derived trip counts)
    // degrade to entry-count-only, exactly LLVM's fail-closed contract.
    let mut derived: FxHashMap<String, FunctionProfile> = FxHashMap::default();
    let mut edge_ok = 0usize;
    let mut drifted_edges = 0usize;
    for f in &m.functions {
        if f.is_declaration || f.blocks.is_empty() {
            continue;
        }
        let h0 = pre_hash_for(&f.name);
        let Some(fp) = profile::get_for_unit_cfg(p, u, &f.name, h0) else {
            continue;
        };
        if fp.post_hash != 0 && fp.post_hash != post_hash_for(&f.name) {
            drifted_edges += 1;
            continue;
        }
        edge_ok += 1;
        let mut copy = fp.clone();
        profile::derive_block_counts(f, &mut copy);
        derived.insert(profile::function_key(u, &f.name), copy);
    }
    if drifted_edges > 0 {
        eprintln!(
            "lccc: PGO: {} function(s) drifted from the training CFG: \
             edge-derived branch/layout/unroll data disabled for them (hot/cold sections kept)",
            drifted_edges
        );
    }
    if !derived.is_empty() {
        let leaked: &'static FxHashMap<String, FunctionProfile> = Box::leak(Box::new(derived));
        DERIVED_PTR.with(|d| d.set(leaked as *const _));
    }
    let _ = edge_ok;
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
        let fp = profile_for_function(p, unit, f)?;
        // Drift gate: never hand a drifted function's stale edge counts to an
        // edge consumer. `total_count` (entry count) is still usable by
        // callers that look it up directly via summary::entry_count_for /
        // get_for_unit; block/edge consumers must degrade to entry-only.
        if fp.post_hash != 0 && fp.post_hash != post_hash_for(&f.name) {
            return None;
        }
        Some(fp)
    })
}

/// The non-drifted, flow-conservation-DERIVED profile for `f`, if one was
/// published by `propagate_profile`. `None` for drifted functions and for
/// functions without an edge-complete profile. Edge consumers (block layout,
/// branch probability, derived trip counts) MUST use this rather than
/// `active_profile_for_function`, so a drifted function is never reordered or
/// given branch/layout decisions from stale edge counts.
pub fn active_derived_profile(f: &IrFunction) -> Option<&'static FunctionProfile> {
    ACTIVE_UNIT.with(|slot| {
        let unit = slot.borrow();
        let unit = unit.as_deref()?;
        let key = profile::function_key(unit, &f.name);
        DERIVED_PTR.with(|d| {
            let p = d.get();
            if p.is_null() {
                None
            } else {
                // SAFETY: leaked once per unit before codegen; only replaced
                // at the next propagate_profile call, never concurrently.
                unsafe { (*p).get(&key) }
            }
        })
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
    let h0 = pre_hash_for(&f.name);
    profile::get_for_unit_cfg(p, u, &f.name, h0)
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
                post_hash: 1,
                entry_label: 0,
                value_sites: Vec::new(),
            },
        );
        p.functions.insert(
            "b::y".into(),
            FunctionProfile {
                total_count: 9,
                block_counts: FxHashMap::default(),
                edge_counts: FxHashMap::default(),
                cfg_hash: 2,
                post_hash: 2,
                entry_label: 0,
                value_sites: Vec::new(),
            },
        );
        assert!(p.is_function_hot("x"));
    }
}
