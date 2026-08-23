//! Dominator-based Global Value Numbering (GVN) pass.
//!
//! This pass assigns "value numbers" to expressions and replaces redundant
//! computations with references to previously computed values (CSE).
//!
//! The pass walks the dominator tree in DFS order with scoped hash tables,
//! so expressions computed in dominating blocks are visible to all dominated
//! blocks. On backtracking, the hash tables are restored to their previous
//! state (same scoping pattern as rename_block in mem2reg).
//!
//! Value-numbered instruction types:
//! - BinOp (with commutative operand canonicalization)
//! - UnaryOp
//! - Cmp
//! - Cast (type-to-type conversions)
//! - GetElementPtr (base + offset address computation)
//! - Load (redundant load elimination within dominator scope, invalidated
//!   by stores, calls, and other memory-clobbering instructions)

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::{
    ConstHashKey, Instruction, IrBinOp, IrCmpOp, IrFunction, IrModule, IrUnaryOp, Operand, Value,
};

/// A value number expression key. Two instructions with the same ExprKey
/// compute the same value (assuming their operands are equivalent).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ExprKey {
    BinOp {
        op: IrBinOp,
        lhs: VNOperand,
        rhs: VNOperand,
        ty: IrType,
    },
    UnaryOp {
        op: IrUnaryOp,
        src: VNOperand,
        ty: IrType,
    },
    Cmp {
        op: IrCmpOp,
        lhs: VNOperand,
        rhs: VNOperand,
        ty: IrType,
    },
    Cast {
        src: VNOperand,
        from_ty: IrType,
        to_ty: IrType,
    },
    Gep {
        base: VNOperand,
        offset: VNOperand,
        ty: IrType,
    },
    /// Address of a named global. `must_mat` is the class-aware CSE split:
    /// foldable (Load/Store ptr / absorbed GEP) vs must-materialize (call
    /// arg, asm, …). Mixing them pins a RIP-foldable `window(%rip)` into a
    /// GPR. Alias tracking still keys on `name` alone.
    GlobalAddr {
        name: String,
        must_mat: bool,
        site_local: Option<u32>,
    },
    /// Load CSE key: two loads from the same pointer with the same type
    /// produce the same value if no intervening memory modification occurs.
    Load { ptr: VNOperand, ty: IrType },
}

/// Returns true if the ExprKey represents a Load (memory-dependent expression).
impl ExprKey {
    fn is_load(&self) -> bool {
        matches!(self, ExprKey::Load { .. })
    }
}

/// A value-numbered operand: either a constant or a value number.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum VNOperand {
    Const(ConstHashKey),
    ValueNum(u32),
}

/// Key for store-to-load forwarding: identifies a memory location by pointer
/// value number and access type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StoreFwdKey {
    ptr_vn: VNOperand,
    ty: IrType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryVersion {
    global: u32,
    base: u64,
}

/// A provably-distinct memory object a pointer may be rooted at.  Load-CSE and
/// store-to-load-forwarding entries keyed on such an object only need to be
/// invalidated by stores through pointers rooted at the SAME object.
///
/// Soundness of the per-object epochs (established in the session-42/45 alias
/// work and re-derived here):
///
/// * two `Alloca`s are distinct frame objects;
/// * an `Alloca` and a `Global` are frame vs static storage;
/// * an `Alloca` and any parameter are distinct — a parameter's value is fixed
///   at function entry, before the callee's frame exists;
/// * a `NoAliasParam` (`restrict` pointer) is distinct from every other object
///   by the C restrict contract (the pointee is accessed only through that
///   parameter and pointers derived from it);
/// * distinct `Global`s are disjoint (the pre-existing GVN assumption; GNU
///   symbol aliases are canonicalized through `GvnContext::canonical`).
///
/// `NoAliasParam`s never alias anything, and `Alloca`s only alias themselves
/// PROVIDED their address cannot be recovered by untracked code.  An alloca is
/// therefore only admitted when `find_nonescaping_allocas` proves its address
/// never escapes (never stored, passed to a call/asm, or used in a
/// terminator): with no escape, every pointer that can reach it is derived
/// in-function from the alloca itself and is tracked here.  Any pointer GVN
/// cannot classify stays `Unknown` (no entry), and stores through it bump the
/// global generation, which invalidates every cached load.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PtrBase {
    Global(String),
    Alloca(u32),
    NoAliasParam(usize),
}

/// Epoch-map key for a `PtrBase`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ObjKey {
    Sym(String),
    Alloca(u32),
    Param(usize),
}

fn obj_key(base: &PtrBase) -> ObjKey {
    match base {
        PtrBase::Global(s) => ObjKey::Sym(s.clone()),
        PtrBase::Alloca(id) => ObjKey::Alloca(*id),
        PtrBase::NoAliasParam(i) => ObjKey::Param(*i),
    }
}

/// Module facts needed to canonicalize GNU symbol aliases during GVN.
#[derive(Debug, Clone, Default)]
pub(crate) struct GvnContext {
    aliases: FxHashMap<String, String>,
}

impl GvnContext {
    pub(crate) fn for_module(module: &IrModule) -> Self {
        let direct: FxHashMap<String, String> = module
            .aliases
            .iter()
            .map(|(alias, target, _)| (alias.clone(), target.clone()))
            .collect();
        let mut aliases = FxHashMap::default();
        for alias in direct.keys() {
            let mut current = alias.as_str();
            let mut seen = FxHashSet::default();
            while let Some(next) = direct.get(current) {
                if !seen.insert(current.to_string()) {
                    break;
                }
                current = next;
            }
            aliases.insert(alias.clone(), current.to_string());
        }
        Self { aliases }
    }

    fn canonical<'a>(&'a self, name: &'a str) -> &'a str {
        self.aliases.get(name).map(String::as_str).unwrap_or(name)
    }
}

/// Mutable state for the GVN pass, threaded through the dominator-tree DFS.
///
/// Groups the value numbering tables, expression maps, and rollback logs that
/// were previously passed as 9 separate `&mut` parameters. The rollback logs
/// enable scoped hash table semantics: on entering a dominator-tree subtree,
/// save the log positions; on backtracking, restore entries to undo changes
/// made in that subtree.
struct GvnState {
    /// Maps Value ID -> value number. Indexed by `Value.0`.
    value_numbers: Vec<u32>,
    /// Next value number to assign.
    next_vn: u32,
    /// Pure expression -> canonical value (not memory-dependent).
    expr_to_value: FxHashMap<ExprKey, Value>,
    /// Load expression -> canonical value and memory version.
    load_expr_to_value: FxHashMap<ExprKey, (Value, MemoryVersion)>,
    /// Canonical value numbers for equivalent GEP addresses. The GEP
    /// instructions themselves are retained, avoiding backend Copy-chain
    /// hazards while letting memory forwarding recognize equal addresses.
    gep_value_numbers: FxHashMap<(VNOperand, VNOperand), u32>,
    /// Generation counter for O(1) load CSE invalidation. When a memory-
    /// clobbering instruction is encountered, bump this counter; cached
    /// load entries with older generations are considered stale.
    load_generation: u32,
    /// Epoch for loads/forwards through pointers with no provably unique base.
    /// Such a pointer may alias any tracked object, so every store (even a
    /// store to a known alloca/global whose own base epoch is precise) must
    /// invalidate unknown-pointer entries. This preserves the optimization win
    /// for distinct known globals while closing the classic STORE-CCP hole.
    unknown_ptr_epoch: u64,
    /// Store-to-load forwarding map with the version recorded after the store.
    store_fwd_map: FxHashMap<StoreFwdKey, (Operand, MemoryVersion)>,
    /// Pointer value number -> provably-distinct base object, for pointers
    /// rooted at a named global, a non-escaping alloca, or a `restrict`
    /// parameter.  Two pointers with different roots never alias, so a store
    /// through one only invalidates cached loads through the same object;
    /// per-object store epochs exploit this (see `PtrBase`).
    ptr_base: FxHashMap<u32, PtrBase>,
    /// Base object -> epoch of its most recent store.  Load CSE and store-to-
    /// load forwarding entries are valid only if they predate the last store
    /// to the SAME object (GEPs within an object may overlap, so any store in
    /// the object invalidates all loads in it; stores to other objects do not).
    base_store_epoch: FxHashMap<ObjKey, u64>,
    next_base_epoch: u64,
    /// Rollback log for per-object epochs.
    base_epoch_log: Vec<(ObjKey, Option<u64>)>,
    /// Non-escaping allocas (computed once per function): only these get
    /// `PtrBase::Alloca` epochs.
    nonescaping_allocas: FxHashSet<u32>,
    /// `restrict` pointer parameter indices: only these get
    /// `PtrBase::NoAliasParam` epochs.
    noalias_params: FxHashSet<usize>,
    context: GvnContext,
    /// Rollback log for `expr_to_value`: (key, previous_value).
    rollback_log: Vec<(ExprKey, Option<Value>)>,
    /// Rollback log for `load_expr_to_value`: (key, previous_entry).
    load_rollback_log: Vec<(ExprKey, Option<(Value, MemoryVersion)>)>,
    /// Rollback log for `store_fwd_map`: (key, previous_entry).
    store_fwd_rollback_log: Vec<(StoreFwdKey, Option<(Operand, MemoryVersion)>)>,
    /// Rollback log for `value_numbers`: (index, previous_vn).
    vn_log: Vec<(usize, u32)>,
    /// Total instructions eliminated across all blocks.
    total_eliminated: usize,
    /// Set of param alloca Value IDs whose address has escaped (used in
    /// non-Load/Store contexts). Store-to-load forwarding is disabled for
    /// these allocas because the backend's ParamRef optimization reads
    /// parameter values from the alloca slot, which may be modified by
    /// stores through aliased pointers.
    escaped_param_allocas: FxHashSet<u32>,
    /// All parameter allocas. Loads from these are never CSE'd or
    /// store-to-load forwarded: the inliner passes arguments by storing into
    /// param allocas and then removes ParamRef+store pairs; rewriting such
    /// loads to a forwarded value would leave an undefined value after
    /// inlining (regression: inlined descending `for(i=n-1;i>=0;i--)` loops
    /// produced i=-1 because the forwarded param load was undefined).
    param_allocas: FxHashSet<u32>,
    /// Volatile allocas must not participate in store-to-load forwarding.
    /// These are used for post-increment/decrement temporaries that must
    /// survive through the stack slot to prevent register coalescing issues.
    volatile_allocas: FxHashSet<u32>,
    /// GlobalAddr dests whose uses force a register (call arg, asm, …).
    /// Populated by `global_addr_cse::classify_must_materialize`.
    must_mat_gaddrs: FxHashSet<u32>,
    /// GlobalAddr values feeding variable-index GEPs must not CSE/hoist: their
    /// site-local identity preserves symbol+index addressing profitability.
    site_local_gaddrs: FxHashSet<u32>,
    /// Canonical value number per global symbol (the first GlobalAddr
    /// numbered for it). Site-local GlobalAddr duplicates get fresh VNs by
    /// design (OP-34), but memory keys must not fork per site.
    symbol_canonical_vn: FxHashMap<String, u32>,
    /// Inverse of the GlobalAddr numbering: VN -> canonical symbol name.
    vn_symbol: FxHashMap<u32, String>,
    /// GEP value number -> canonical address VN. Two GEPs over the same
    /// symbol with equal offsets denote the same address even when their
    /// GlobalAddr bases stayed site-local with distinct VNs. This folds
    /// those GEP VNs onto one canonical address VN for load-CSE and
    /// store-forwarding keys. The GEP instructions and site-local
    /// GlobalAddrs are untouched, so SIB addressing decisions are
    /// unaffected (OP-12/OP-32).
    gep_addr_canonical: FxHashMap<u32, u32>,
    /// (canonical symbol base VN, offset VN, type) -> canonical address VN.
    addr_key_to_vn: FxHashMap<(u32, VNOperand, IrType), u32>,
}

impl GvnState {
    /// Create a new GVN state sized for `max_value_id` values.
    fn new(
        max_value_id: usize,
        escaped_param_allocas: FxHashSet<u32>,
        param_allocas: FxHashSet<u32>,
        volatile_allocas: FxHashSet<u32>,
        context: &GvnContext,
        must_mat_gaddrs: FxHashSet<u32>,
        site_local_gaddrs: FxHashSet<u32>,
        nonescaping_allocas: FxHashSet<u32>,
        noalias_params: FxHashSet<usize>,
    ) -> Self {
        Self {
            value_numbers: vec![u32::MAX; max_value_id + 1],
            next_vn: 0,
            expr_to_value: FxHashMap::default(),
            load_expr_to_value: FxHashMap::default(),
            gep_value_numbers: FxHashMap::default(),
            load_generation: 0,
            unknown_ptr_epoch: 0,
            store_fwd_map: FxHashMap::default(),
            ptr_base: FxHashMap::default(),
            base_store_epoch: FxHashMap::default(),
            next_base_epoch: 0,
            base_epoch_log: Vec::new(),
            nonescaping_allocas,
            noalias_params,
            context: context.clone(),
            rollback_log: Vec::new(),
            load_rollback_log: Vec::new(),
            store_fwd_rollback_log: Vec::new(),
            vn_log: Vec::new(),
            total_eliminated: 0,
            escaped_param_allocas,
            param_allocas,
            volatile_allocas,
            must_mat_gaddrs,
            site_local_gaddrs,
            symbol_canonical_vn: FxHashMap::default(),
            vn_symbol: FxHashMap::default(),
            gep_addr_canonical: FxHashMap::default(),
            addr_key_to_vn: FxHashMap::default(),
        }
    }

    /// Assign a fresh value number, returning it.
    fn fresh_vn(&mut self) -> u32 {
        let vn = self.next_vn;
        self.next_vn += 1;
        vn
    }

    /// Assign a fresh value number to `dest` and record it in the rollback log.
    fn assign_fresh_vn(&mut self, dest: Value) {
        let vn = self.fresh_vn();
        let idx = dest.0 as usize;
        if idx < self.value_numbers.len() {
            let old_vn = self.value_numbers[idx];
            self.vn_log.push((idx, old_vn));
            self.value_numbers[idx] = vn;
        }
    }

    /// Convert an Operand to a VNOperand for hashing.
    /// If the value hasn't been assigned a value number yet (e.g. a function
    /// parameter or an alloca whose definition appears later in the block),
    /// assign it a fresh unique VN on the spot to avoid collisions between
    /// different un-numbered values and already-assigned VNs.
    fn operand_to_vn(&mut self, op: &Operand) -> VNOperand {
        match op {
            Operand::Const(c) => VNOperand::Const(c.to_hash_key()),
            Operand::Value(v) => {
                let idx = v.0 as usize;
                // Ensure the table is large enough
                if idx >= self.value_numbers.len() {
                    self.value_numbers.resize(idx + 1, u32::MAX);
                }
                if self.value_numbers[idx] != u32::MAX {
                    VNOperand::ValueNum(self.value_numbers[idx])
                } else {
                    // Assign a fresh VN to this previously un-numbered value
                    let vn = self.fresh_vn();
                    let old_vn = self.value_numbers[idx];
                    self.vn_log.push((idx, old_vn));
                    self.value_numbers[idx] = vn;
                    VNOperand::ValueNum(vn)
                }
            }
        }
    }

    /// Try to create an ExprKey for an instruction (for value numbering).
    /// Returns the expression key and the destination value, or None if
    /// the instruction is not eligible for value numbering.
    /// A cached load/store-forwarding entry (created at `gen`) is valid iff no
    /// global memory clobber and no store to the SAME global symbol happened
    /// since. Stores to other (disjoint) globals do not invalidate it.
    fn memory_version(&self, ptr_vn: &VNOperand) -> MemoryVersion {
        let base = match ptr_vn {
            VNOperand::ValueNum(v) => match self.ptr_base.get(v) {
                Some(base) => self
                    .base_store_epoch
                    .get(&obj_key(base))
                    .copied()
                    .unwrap_or(0),
                None => self.unknown_ptr_epoch,
            },
            VNOperand::Const(_) => self.unknown_ptr_epoch,
        };
        MemoryVersion {
            global: self.load_generation,
            base,
        }
    }

    fn entry_valid_for(&self, ptr_vn: &VNOperand, version: MemoryVersion) -> bool {
        version == self.memory_version(ptr_vn)
    }

    /// Resolve an operand's base object, or `None` when it is unclassified.
    /// Only consults already-assigned value numbers (fail closed on
    /// un-numbered values — those get no epoch and stay conservative).
    fn base_of(&self, op: &Operand) -> Option<PtrBase> {
        let Operand::Value(v) = op else {
            return None;
        };
        let idx = v.0 as usize;
        if idx >= self.value_numbers.len() || self.value_numbers[idx] == u32::MAX {
            return None;
        }
        self.ptr_base.get(&self.value_numbers[idx]).cloned()
    }

    /// Propagate `src`'s base object to `dest`'s value number, if any.
    fn propagate_ptr_base(&mut self, src: Value, dest: Value) {
        let src_vn = match self.operand_to_vn(&Operand::Value(src)) {
            VNOperand::ValueNum(vn) => vn,
            VNOperand::Const(_) => return,
        };
        let d_idx = dest.0 as usize;
        if d_idx >= self.value_numbers.len() || self.value_numbers[d_idx] == u32::MAX {
            return;
        }
        if let Some(base) = self.ptr_base.get(&src_vn).cloned() {
            self.ptr_base.insert(self.value_numbers[d_idx], base);
        }
    }

    /// Record `inst`'s destination pointer's base object so later stores can
    /// invalidate only that object's cached loads.  Roots are `GlobalAddr`,
    /// non-escaping `Alloca`, and `restrict` `ParamRef`; the base propagates
    /// through single-source address-preserving operations (`GetElementPtr`,
    /// pointer-to-pointer `Cast`, `Copy`) and through integer-width pointer
    /// arithmetic (`Add` with exactly one rooted operand, `Sub` with a rooted
    /// left operand).  Every other shape — phis/selects with mixed bases,
    /// loads of pointers, non-pointer casts, two-pointer arithmetic — stays
    /// `Unknown` and falls back to the global generation.
    fn track_ptr_base(&mut self, inst: &Instruction) {
        match inst {
            Instruction::GlobalAddr { dest, name } => {
                let idx = dest.0 as usize;
                if idx < self.value_numbers.len() && self.value_numbers[idx] != u32::MAX {
                    self.ptr_base.insert(
                        self.value_numbers[idx],
                        PtrBase::Global(self.context.canonical(name).to_string()),
                    );
                }
            }
            Instruction::Alloca { dest, .. } => {
                if self.nonescaping_allocas.contains(&dest.0) {
                    let idx = dest.0 as usize;
                    if idx < self.value_numbers.len() && self.value_numbers[idx] != u32::MAX {
                        self.ptr_base
                            .insert(self.value_numbers[idx], PtrBase::Alloca(dest.0));
                    }
                }
            }
            Instruction::ParamRef {
                dest, param_idx, ty, ..
            } => {
                if *ty == IrType::Ptr && self.noalias_params.contains(param_idx) {
                    let idx = dest.0 as usize;
                    if idx < self.value_numbers.len() && self.value_numbers[idx] != u32::MAX {
                        self.ptr_base.insert(
                            self.value_numbers[idx],
                            PtrBase::NoAliasParam(*param_idx),
                        );
                    }
                }
            }
            Instruction::GetElementPtr { dest, base, .. } => {
                self.propagate_ptr_base(*base, *dest);
            }
            Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } => {
                self.propagate_ptr_base(*src, *dest);
            }
            Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } if *from_ty == IrType::Ptr && *to_ty == IrType::Ptr => {
                if let Operand::Value(src) = src {
                    self.propagate_ptr_base(*src, *dest);
                }
            }
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ..
            } => {
                let base = match (self.base_of(lhs), self.base_of(rhs)) {
                    (Some(b), None) | (None, Some(b)) => Some(b),
                    _ => None,
                };
                if let Some(base) = base {
                    let d_idx = dest.0 as usize;
                    if d_idx < self.value_numbers.len()
                        && self.value_numbers[d_idx] != u32::MAX
                    {
                        self.ptr_base.insert(self.value_numbers[d_idx], base);
                    }
                }
            }
            Instruction::BinOp {
                dest,
                op: IrBinOp::Sub,
                lhs,
                rhs,
                ..
            } => {
                // `ptr - offset` preserves the base; `ptr - ptr` (a distance)
                // and `offset - ptr` (invalid C) do not.
                if let (Some(base), None) = (self.base_of(lhs), self.base_of(rhs)) {
                    let d_idx = dest.0 as usize;
                    if d_idx < self.value_numbers.len()
                        && self.value_numbers[d_idx] != u32::MAX
                    {
                        self.ptr_base.insert(self.value_numbers[d_idx], base);
                    }
                }
            }
            _ => {}
        }
    }

    fn make_expr_key(&mut self, inst: &Instruction) -> Option<(ExprKey, Value)> {
        match inst {
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                // Don't CSE 128-bit BinOps. The backend represents 128-bit
                // values in XMM register pairs; a `Copy` between two such
                // values can be misallocated (stale register on reload),
                // producing wrong results for vector/PCLMUL CRC code.
                if ty.is_128bit() {
                    self.assign_fresh_vn(*dest);
                    return None;
                }
                let lhs_vn = self.operand_to_vn(lhs);
                let rhs_vn = self.operand_to_vn(rhs);

                // For commutative operations, canonicalize operand order
                let (lhs_vn, rhs_vn) = if op.is_commutative() {
                    canonical_order(lhs_vn, rhs_vn)
                } else {
                    (lhs_vn, rhs_vn)
                };

                Some((
                    ExprKey::BinOp {
                        op: *op,
                        lhs: lhs_vn,
                        rhs: rhs_vn,
                        ty: *ty,
                    },
                    *dest,
                ))
            }
            Instruction::UnaryOp { dest, op, src, ty } => {
                if ty.is_128bit() {
                    self.assign_fresh_vn(*dest);
                    return None;
                }
                let src_vn = self.operand_to_vn(src);
                Some((
                    ExprKey::UnaryOp {
                        op: *op,
                        src: src_vn,
                        ty: *ty,
                    },
                    *dest,
                ))
            }
            Instruction::Cmp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } => {
                let lhs_vn = self.operand_to_vn(lhs);
                let rhs_vn = self.operand_to_vn(rhs);
                Some((
                    ExprKey::Cmp {
                        op: *op,
                        lhs: lhs_vn,
                        rhs: rhs_vn,
                        ty: *ty,
                    },
                    *dest,
                ))
            }
            Instruction::Cast {
                dest,
                src,
                from_ty,
                to_ty,
            } => {
                // Don't CSE casts to/from 128-bit types (complex codegen)
                if from_ty.is_128bit() || to_ty.is_128bit() {
                    return None;
                }
                let src_vn = self.operand_to_vn(src);
                Some((
                    ExprKey::Cast {
                        src: src_vn,
                        from_ty: *from_ty,
                        to_ty: *to_ty,
                    },
                    *dest,
                ))
            }
            Instruction::GetElementPtr {
                dest,
                base,
                offset,
                ty,
            } => {
                // re-enable GEP CSE. Two GEPs with the same base and
                // offset compute the same address; the duplicate becomes a
                // Copy that the copy-coalescer folds to one home slot (the
                // nbody inner-loop recomputes `&bodies[i]` once per field
                // access, and each recompute paid a leaq + a scratch-slot
                // spill). The historical "stale base register at fold points"
                // defect was in the pre-register-aware codegen; the current
                // Copy/slot machinery keeps GEP results in a single coalesced
                // slot, and the store-forwarding analysis keys on value
                // numbers, which CSE only strengthens.
                let base_vn = self.operand_to_vn(&Operand::Value(*base));
                let offset_vn = self.operand_to_vn(offset);
                Some((
                    ExprKey::Gep {
                        base: base_vn,
                        offset: offset_vn,
                        ty: *ty,
                    },
                    *dest,
                ))
            }
            // Pure: a symbol's address is constant for the process lifetime.
            Instruction::GlobalAddr { dest, name } => Some((
                ExprKey::GlobalAddr {
                    name: self.context.canonical(name).to_string(),
                    must_mat: self.must_mat_gaddrs.contains(&dest.0),
                    site_local: self.site_local_gaddrs.contains(&dest.0).then_some(dest.0),
                },
                *dest,
            )),
            // Load CSE: two loads from the same pointer with the same type can be
            // CSE'd if no intervening memory modification occurred. The caller
            // (process_block) handles invalidating Load entries on memory clobbers.
            //
            // Excluded from CSE:
            // - Segment-overridden loads: access thread-local or CPU-local storage
            //   that may differ between accesses even without visible stores
            // - Float, long double, i128 types: use different register paths in
            //   codegen that complicate Copy instruction handling
            // - AtomicLoad: has ordering semantics (falls through to _ => None)
            Instruction::Load {
                dest,
                ptr,
                ty,
                seg_override,
                volatile,
            } => {
                if *seg_override != AddressSpace::Default {
                    return None;
                }
                // Volatile loads are observable side effects: no CSE, no
                // forwarding (each access must execute exactly once).
                if *volatile {
                    return None;
                }
                // F32/F64 load CSE is ENABLED (session-25).  The session-23
                // revert blamed GVN-created FP Copies for the fp_die_at_birth
                // miscompile (chain_div returning chain_neg's value), but the
                // true root cause was `folded_gep_values` leaking across
                // functions in CodegenState::reset_for_function: skipped-GEP
                // IDs from a load-heavy function collided with value IDs of a
                // LATER function and fired spurious rematerialisations there
                // — which is why the symptom looked like "unrelated
                // functions" perturbing each other.  With the per-function
                // reset in place, FP copies created by CSE are safe.
                // Long double (x87) and 128-bit loads stay excluded: their
                // lowering uses multi-register/x87 paths that CSE's plain
                // Copy does not model.
                if ty.is_long_double() || ty.is_128bit() {
                    return None;
                }
                // Never CSE/forward loads from parameter allocas (see the
                // param_allocas field doc).
                if self.param_allocas.contains(&ptr.0) {
                    return None;
                }
                let ptr_vn = self.operand_to_vn(&Operand::Value(*ptr));
                let ptr_vn = self.canonical_addr_vn(ptr_vn);
                Some((
                    ExprKey::Load {
                        ptr: ptr_vn,
                        ty: *ty,
                    },
                    *dest,
                ))
            }
            // Other instructions (Store, Call, AtomicLoad, etc.) are not eligible.
            // AtomicLoad is excluded because it has memory ordering semantics that
            // require the load to actually execute.
            _ => None,
        }
    }

    /// Fold a pointer VN onto its canonical address VN when it is a GEP over
    /// a symbol whose sites were deliberately kept distinct.
    fn canonical_addr_vn(&self, vn: VNOperand) -> VNOperand {
        if let VNOperand::ValueNum(v) = vn {
            if let Some(&c) = self.gep_addr_canonical.get(&v) {
                return VNOperand::ValueNum(c);
            }
        }
        vn
    }

    /// Canonical base VN when `base` resolves to a GlobalAddr (through
    /// Copies): the first VN numbered for that symbol.
    fn canonical_symbol_base_vn(&mut self, base: &Value) -> Option<u32> {
        let vn = match self.operand_to_vn(&Operand::Value(*base)) {
            VNOperand::ValueNum(v) => v,
            VNOperand::Const(_) => return None,
        };
        let sym = self.vn_symbol.get(&vn)?.clone();
        self.symbol_canonical_vn.get(&sym).copied()
    }

    /// Record a freshly numbered GlobalAddr's symbol identity.
    fn record_symbol_vn(&mut self, name: &str, vn: u32) {
        let canon = self.context.canonical(name).to_string();
        self.vn_symbol.insert(vn, canon.clone());
        self.symbol_canonical_vn.entry(canon).or_insert(vn);
    }

    /// Record a freshly numbered GEP under its canonical address key so
    /// later loads/stores through an equivalent GEP (possibly over a
    /// site-local GlobalAddr duplicate) share one memory key.
    fn record_gep_addr_key(&mut self, base: &Value, offset: &Operand, ty: IrType, vn: u32) {
        let Some(cb) = self.canonical_symbol_base_vn(base) else {
            return;
        };
        let off_vn = self.operand_to_vn(offset);
        let key = (cb, off_vn, ty);
        match self.addr_key_to_vn.get(&key).copied() {
            Some(c) => {
                self.gep_addr_canonical.insert(vn, c);
            }
            None => {
                self.addr_key_to_vn.insert(key, vn);
                self.gep_addr_canonical.insert(vn, vn);
            }
        }
    }

    /// Save the current log positions for later rollback.
    fn save_scope(&self) -> ScopeCheckpoint {
        ScopeCheckpoint {
            rollback_start: self.rollback_log.len(),
            load_rollback_start: self.load_rollback_log.len(),
            store_fwd_rollback_start: self.store_fwd_rollback_log.len(),
            vn_log_start: self.vn_log.len(),
            base_epoch_log_start: self.base_epoch_log.len(),
            saved_load_generation: self.load_generation,
            saved_unknown_ptr_epoch: self.unknown_ptr_epoch,
        }
    }

    /// Restore state to a previously saved checkpoint, undoing all changes
    /// made since the checkpoint was taken.
    fn restore_scope(&mut self, checkpoint: &ScopeCheckpoint) {
        // Rollback: restore expr_to_value
        while self.rollback_log.len() > checkpoint.rollback_start {
            let (key, old_val) = self
                .rollback_log
                .pop()
                .expect("rollback_log length checked by while condition");
            if let Some(val) = old_val {
                self.expr_to_value.insert(key, val);
            } else {
                self.expr_to_value.remove(&key);
            }
        }

        // Rollback: restore load_expr_to_value
        while self.load_rollback_log.len() > checkpoint.load_rollback_start {
            let (key, old_val) = self
                .load_rollback_log
                .pop()
                .expect("load_rollback_log length checked by while condition");
            if let Some(val) = old_val {
                self.load_expr_to_value.insert(key, val);
            } else {
                self.load_expr_to_value.remove(&key);
            }
        }

        // Rollback: restore store_fwd_map
        while self.store_fwd_rollback_log.len() > checkpoint.store_fwd_rollback_start {
            let (key, old_val) = self
                .store_fwd_rollback_log
                .pop()
                .expect("store_fwd_rollback_log length checked by while condition");
            if let Some(val) = old_val {
                self.store_fwd_map.insert(key, val);
            } else {
                self.store_fwd_map.remove(&key);
            }
        }

        // Rollback: restore value_numbers
        while self.vn_log.len() > checkpoint.vn_log_start {
            let (idx, old_vn) = self
                .vn_log
                .pop()
                .expect("vn_log length checked by while condition");
            self.value_numbers[idx] = old_vn;
        }

        // Rollback: restore base_store_epoch entries pushed since the checkpoint
        while self.base_epoch_log.len() > checkpoint.base_epoch_log_start {
            let (sym, old) = self
                .base_epoch_log
                .pop()
                .expect("base_epoch_log length checked by while condition");
            match old {
                Some(e) => {
                    self.base_store_epoch.insert(sym, e);
                }
                None => {
                    self.base_store_epoch.remove(&sym);
                }
            }
        }

        // Rollback: restore memory epochs
        self.load_generation = checkpoint.saved_load_generation;
        self.unknown_ptr_epoch = checkpoint.saved_unknown_ptr_epoch;
    }
}

/// Saved positions in the rollback logs, used by `GvnState::save_scope` /
/// `GvnState::restore_scope` to implement scoped hash table semantics.
struct ScopeCheckpoint {
    rollback_start: usize,
    load_rollback_start: usize,
    store_fwd_rollback_start: usize,
    vn_log_start: usize,
    base_epoch_log_start: usize,
    saved_load_generation: u32,
    saved_unknown_ptr_epoch: u64,
}

/// Find param allocas whose address has escaped (used in non-Load/Store contexts).
///
/// When a param alloca's address is taken (e.g., via `&x` which becomes a GEP
/// then gets simplified to a Copy), the alloca can be modified through aliased
/// pointers. Store-to-load forwarding must be disabled for these allocas because
/// the backend's ParamRef optimization reads parameter values from the alloca
/// slot at point of use, not at point of definition. If an aliased store
/// modifies the alloca between the original store and the forwarded use, the
/// read will return the wrong value.
fn find_escaped_param_allocas(func: &IrFunction) -> FxHashSet<u32> {
    let param_alloca_set: FxHashSet<u32> = func.param_alloca_values.iter().map(|v| v.0).collect();
    if param_alloca_set.is_empty() {
        return FxHashSet::default();
    }

    let mut escaped = FxHashSet::default();

    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                // Load from param alloca is fine - it reads the value
                Instruction::Load { ptr, .. } => {
                    // ptr is the alloca value itself, this is a normal use
                    let _ = ptr;
                }
                // Store TO param alloca is fine - it writes to the alloca
                Instruction::Store { ptr, val, .. } => {
                    // ptr is the alloca, this is fine
                    let _ = ptr;
                    // But if the alloca value is used as a stored VALUE
                    // (i.e., its address is being stored somewhere), it escapes
                    if let Operand::Value(v) = val {
                        if param_alloca_set.contains(&v.0) {
                            escaped.insert(v.0);
                        }
                    }
                }
                // Copy of param alloca = address taken (e.g., simplified &x GEP)
                Instruction::Copy {
                    src: Operand::Value(v),
                    ..
                } => {
                    if param_alloca_set.contains(&v.0) {
                        escaped.insert(v.0);
                    }
                }
                // Any other instruction using the alloca value means escape
                _ => {
                    inst.for_each_used_value(|vid| {
                        if param_alloca_set.contains(&vid) {
                            escaped.insert(vid);
                        }
                    });
                }
            }
        }
    }

    escaped
}

/// All parameter allocas (first `num_params` allocas in the entry block).
fn find_param_allocas(func: &IrFunction) -> FxHashSet<u32> {
    func.param_alloca_values.iter().map(|v| v.0).collect()
}

/// Find volatile allocas in the function. These must not participate in
/// store-to-load forwarding because they are used to preserve values that
/// would otherwise be lost to register coalescing (e.g., post-decrement
/// return values in `while(n--)` patterns).
fn find_volatile_allocas(func: &IrFunction) -> FxHashSet<u32> {
    let mut volatile = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile: true,
                ..
            } = inst
            {
                volatile.insert(dest.0);
            }
        }
    }
    volatile
}

fn function_uses_128(func: &IrFunction) -> bool {
    func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| match i {
            Instruction::BinOp { ty, .. }
            | Instruction::UnaryOp { ty, .. }
            | Instruction::Load { ty, .. }
            | Instruction::Store { ty, .. }
            | Instruction::Phi { ty, .. }
            | Instruction::Select { ty, .. } => ty.is_128bit(),
            Instruction::Cast { from_ty, to_ty, .. } => from_ty.is_128bit() || to_ty.is_128bit(),
            Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                info.return_type.is_128bit() || info.arg_types.iter().any(|t| t.is_128bit())
            }
            Instruction::InlineAsm { operand_types, .. } => {
                operand_types.iter().any(|t| t.is_128bit())
            }
            _ => false,
        })
    })
}

/// Run dominator-based GVN on a single function.
pub(crate) fn run_gvn_function(func: &mut IrFunction) -> usize {
    run_gvn_function_with_context(func, &GvnContext::default())
}

/// Allocas whose address provably never escapes the function.
///
/// The owner-set fixpoint propagates every alloca's address through
/// address-preserving operations (GEP, Copy, Cast, BinOp/UnaryOp integer
/// folding, Phi, Select); an alloca ESCAPES as soon as any value carrying its
/// address is stored as data, passed to a call/asm, used in a terminator, or
/// written by an atomic.  Only non-escaping allocas are safe for
/// `PtrBase::Alloca` epochs: if the address cannot leave the function, every
/// pointer that can reach the alloca is tracked by GVN and carries its epoch.
///
/// The fixpoint deliberately over-approximates address carriers (an int-folded
/// address still counts), so a root is never falsely reported non-escaping.
fn find_nonescaping_allocas(func: &IrFunction) -> FxHashSet<u32> {
    let mut roots: FxHashSet<u32> = FxHashSet::default();
    let mut owners: FxHashMap<u32, FxHashSet<u32>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                roots.insert(dest.0);
                let mut s = FxHashSet::default();
                s.insert(dest.0);
                owners.insert(dest.0, s);
            }
        }
    }
    if owners.is_empty() {
        return roots;
    }

    // Propagate owner sets through address-preserving operations to a fixpoint.
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let Some(dest) = inst.dest() else { continue };
                if !matches!(
                    inst,
                    Instruction::GetElementPtr { .. }
                        | Instruction::Copy { .. }
                        | Instruction::Cast { .. }
                        | Instruction::BinOp { .. }
                        | Instruction::UnaryOp { .. }
                        | Instruction::Phi { .. }
                        | Instruction::Select { .. }
                ) {
                    continue;
                }
                let mut incoming: FxHashSet<u32> = FxHashSet::default();
                inst.for_each_used_value(|v| {
                    if let Some(o) = owners.get(&v) {
                        incoming.extend(o.iter().copied());
                    }
                });
                if incoming.is_empty() {
                    continue;
                }
                let entry = owners.entry(dest.0).or_default();
                for o in incoming {
                    if entry.insert(o) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut escaped: FxHashSet<u32> = FxHashSet::default();
    fn mark_value(
        v: u32,
        owners: &FxHashMap<u32, FxHashSet<u32>>,
        escaped: &mut FxHashSet<u32>,
    ) {
        if let Some(o) = owners.get(&v) {
            escaped.extend(o.iter().copied());
        }
    }
    fn mark_operand(
        op: &Operand,
        owners: &FxHashMap<u32, FxHashSet<u32>>,
        escaped: &mut FxHashSet<u32>,
    ) {
        if let Operand::Value(v) = op {
            mark_value(v.0, owners, escaped);
        }
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Store { val, .. } => mark_operand(val, &owners, &mut escaped),
                Instruction::AtomicStore { val, .. }
                | Instruction::AtomicRmw { val, .. } => mark_operand(val, &owners, &mut escaped),
                Instruction::AtomicCmpxchg {
                    expected, desired, ..
                } => {
                    mark_operand(expected, &owners, &mut escaped);
                    mark_operand(desired, &owners, &mut escaped);
                }
                Instruction::Call { .. }
                | Instruction::CallIndirect { .. }
                | Instruction::InlineAsm { .. }
                | Instruction::StackRestore { .. } => {
                    inst.for_each_used_value(|v| mark_value(v, &owners, &mut escaped));
                }
                _ => {}
            }
        }
        block
            .terminator
            .for_each_used_value(|v| mark_value(v, &owners, &mut escaped));
    }

    roots.retain(|r| !escaped.contains(r));
    roots
}

/// `restrict` pointer parameter indices (their `IrParam.noalias` flag).
fn find_noalias_params(func: &IrFunction) -> FxHashSet<usize> {
    func.params
        .iter()
        .enumerate()
        .filter(|(_, p)| p.noalias)
        .map(|(i, _)| i)
        .collect()
}

pub(crate) fn run_gvn_function_with_context(func: &mut IrFunction, context: &GvnContext) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks == 0 || function_uses_128(func) {
        return 0;
    }
    let escaped = find_escaped_param_allocas(func);
    let volatile = find_volatile_allocas(func);
    let nonescaping = find_nonescaping_allocas(func);
    let noalias = find_noalias_params(func);
    if num_blocks == 1 {
        let mut state = GvnState::new(
            func.max_value_id() as usize,
            escaped,
            find_param_allocas(func),
            volatile,
            context,
            super::global_addr_cse::classify_must_materialize(func),
            super::global_addr_cse::classify_site_local_indexed(func),
            nonescaping,
            noalias,
        );
        return process_block(0, func, &mut state);
    }
    let cfg = analysis::CfgAnalysis::build(func);
    run_gvn_with_analysis_and_context(func, &cfg, context)
}

/// Run GVN using pre-computed CFG analysis.
pub(crate) fn run_gvn_with_analysis(func: &mut IrFunction, cfg: &analysis::CfgAnalysis) -> usize {
    run_gvn_with_analysis_and_context(func, cfg, &GvnContext::default())
}

pub(crate) fn run_gvn_with_analysis_and_context(
    func: &mut IrFunction,
    cfg: &analysis::CfgAnalysis,
    context: &GvnContext,
) -> usize {
    let num_blocks = func.blocks.len();
    if num_blocks == 0 || function_uses_128(func) {
        return 0;
    }
    let escaped = find_escaped_param_allocas(func);
    let volatile = find_volatile_allocas(func);
    let must_mat = super::global_addr_cse::classify_must_materialize(func);
    let site_local = super::global_addr_cse::classify_site_local_indexed(func);
    let nonescaping = find_nonescaping_allocas(func);
    let noalias = find_noalias_params(func);
    if num_blocks == 1 {
        let mut state = GvnState::new(
            func.max_value_id() as usize,
            escaped,
            find_param_allocas(func),
            volatile,
            context,
            must_mat,
            site_local,
            nonescaping,
            noalias,
        );
        return process_block(0, func, &mut state);
    }
    let mut state = GvnState::new(
        func.max_value_id() as usize,
        escaped,
        find_param_allocas(func),
        volatile,
        context,
        must_mat,
        site_local,
        nonescaping,
        noalias,
    );
    gvn_dfs(0, func, &cfg.dom_children, &cfg.preds, &mut state);
    state.total_eliminated
}

/// Recursive DFS over the dominator tree for GVN.
/// Processes block_idx, then recurses into dominated children.
/// Uses rollback logs to restore state on backtracking.
fn gvn_dfs(
    block_idx: usize,
    func: &mut IrFunction,
    dom_children: &[Vec<usize>],
    preds: &analysis::FlatAdj,
    state: &mut GvnState,
) {
    let checkpoint = state.save_scope();

    // At block entry, decide whether to invalidate inherited Load CSE entries.
    // Load CSE across blocks is safe when this block has exactly one CFG
    // predecessor (straight-line code). At merge points (multiple predecessors),
    // conservatively invalidate all Load entries because a non-dominating
    // predecessor may have stored to memory, making cached loads stale.
    //
    // Invalidation is O(1): just bump load_generation. Entries with older
    // generations are ignored during lookup.
    if block_idx != 0 && preds.len(block_idx) > 1 {
        state.load_generation += 1;
    }

    // Process instructions in this block
    let eliminated = process_block(block_idx, func, state);
    state.total_eliminated += eliminated;

    // Recurse into dominator tree children.
    // Iterate by index to avoid cloning the children Vec.
    let num_children = dom_children[block_idx].len();
    for ci in 0..num_children {
        let child = dom_children[block_idx][ci];
        gvn_dfs(child, func, dom_children, preds, state);
    }

    state.restore_scope(&checkpoint);
}

/// Check if an instruction may modify memory, invalidating cached load values.
/// This is conservative: any instruction that could write to memory or call
/// external code (which could write to memory) returns true.
fn clobbers_memory(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Store { .. }
            | Instruction::Call { .. }
            | Instruction::CallIndirect { .. }
            | Instruction::Memcpy { .. }
            | Instruction::AtomicRmw { .. }
            | Instruction::AtomicInc { .. }
            | Instruction::AtomicCmpxchg { .. }
            | Instruction::AtomicStore { .. }
            | Instruction::Fence { .. }
            | Instruction::InlineAsm { .. }
            | Instruction::VaStart { .. }
            | Instruction::VaEnd { .. }
            | Instruction::VaCopy { .. }
    ) || matches!(
        inst,
        Instruction::Intrinsic {
            dest_ptr: Some(_),
            ..
        }
    )
}

/// Check if a Store instruction is eligible for store-to-load forwarding.
/// Same restrictions as Load CSE (session-25): no segment overrides, no
/// long-double/i128 types. F32/F64 stores ARE forwardable — FP load CSE is
/// enabled and the forwarding path creates the same backend Copy machinery
/// (kept in the XMM domain) that FP load CSE already proven-safe uses.
fn is_forwardable_store(inst: &Instruction) -> bool {
    match inst {
        Instruction::Store {
            ty, seg_override, ..
        } => *seg_override == AddressSpace::Default && !ty.is_long_double() && !ty.is_128bit(),
        _ => false,
    }
}

/// Process a single basic block for GVN.
/// Returns the number of instructions eliminated.
///
/// Load CSE entries are stored separately in `state.load_expr_to_value`, tagged
/// with a generation counter for O(1) invalidation on memory clobber. Cross-block
/// Load CSE propagation is controlled by `gvn_dfs` which invalidates Load
/// entries at merge points (blocks with multiple CFG predecessors).
///
/// Store-to-load forwarding: when a Store writes value V to pointer P, subsequent
/// Loads from P (same VN, same type, no intervening memory clobber) are replaced
/// with Copy(V). This eliminates redundant loads after stores, a common pattern
/// in struct initialization, local variable access, etc.
fn process_block(block_idx: usize, func: &mut IrFunction, state: &mut GvnState) -> usize {
    let mut eliminated = 0;
    let mut new_instructions = Vec::with_capacity(func.blocks[block_idx].instructions.len());
    // Same-block membership used to scan every preceding instruction for every
    // candidate expression. Track definitions explicitly to make large generated
    // basic blocks O(n) instead of O(n^2) without changing CSE legality.
    let mut block_defs: FxHashSet<u32> = FxHashSet::default();
    // GVN replaces instructions 1:1 (original or Copy), so spans stay parallel
    let new_spans = std::mem::take(&mut func.blocks[block_idx].source_spans);

    for inst in func.blocks[block_idx].instructions.drain(..) {
        // Memory invalidation: true clobbers (calls, atomics, asm, ...) and
        // stores to unknown/seg-overridden pointers bump the global generation
        // (conservative). Stores to a known global only advance that symbol's
        // epoch: distinct globals never alias, so other globals' loads stay
        // cached — this keeps gzip's copy_block `outcnt` in a register across
        // `outbuf` stores.
        if let Instruction::Store {
            ptr, seg_override, ..
        } = &inst
        {
            // Unknown-base pointer loads/forwards may alias any store.  Bump a
            // separate epoch on every store so a cached `load *p` where `p` is
            // a phi of multiple roots cannot survive a later `store &b` just
            // because that store also has a precise per-base epoch.
            state.unknown_ptr_epoch = state.unknown_ptr_epoch.saturating_add(1);
            if *seg_override != AddressSpace::Default {
                state.load_generation += 1;
            } else {
                let pv = state.operand_to_vn(&Operand::Value(*ptr));
                if let VNOperand::ValueNum(pvn) = pv {
                    match state.ptr_base.get(&pvn) {
                        Some(base) => {
                            if std::env::var_os("CCC_DEBUG_GVN").is_some() {
                                eprintln!("[GVNDBG] store bumps epoch of {:?}", base);
                            }
                            let key = obj_key(base);
                            state.next_base_epoch = state.next_base_epoch.saturating_add(1);
                            let old = state
                                .base_store_epoch
                                .insert(key.clone(), state.next_base_epoch);
                            state.base_epoch_log.push((key, old));
                        }
                        None => {
                            if std::env::var_os("CCC_DEBUG_GVN").is_some() {
                                eprintln!("[GVNDBG] store UNKNOWN base -> global gen++");
                            }
                            state.load_generation += 1;
                        }
                    }
                } else {
                    if std::env::var_os("CCC_DEBUG_GVN").is_some() {
                        eprintln!("[GVNDBG] store CONST base -> global gen++");
                    }
                    state.load_generation += 1;
                }
            }
        } else if clobbers_memory(&inst) {
            state.load_generation += 1;
        }

        // Store-to-load forwarding: record stored values for subsequent loads.
        // This happens AFTER the invalidation step (a store to a known symbol
        // advances that symbol's epoch without touching the global generation),
        // so the stored value is recorded at the current generation and remains
        // visible to subsequent loads of the same pointer.
        //
        // Skip forwarding for stores to escaped param allocas: the backend's
        // ParamRef optimization reads from the alloca slot at point of use, so
        // forwarding a ParamRef through an escaped alloca is unsound when an
        // aliased pointer may write to the same slot.
        if is_forwardable_store(&inst) {
            if let Instruction::Store { val, ptr, ty, .. } = &inst {
                if !state.escaped_param_allocas.contains(&ptr.0)
                    && !state.volatile_allocas.contains(&ptr.0)
                {
                    let ptr_vn = state.operand_to_vn(&Operand::Value(*ptr));
                    let ptr_vn = state.canonical_addr_vn(ptr_vn);
                    let version = state.memory_version(&ptr_vn);
                    let fwd_key = StoreFwdKey { ptr_vn, ty: *ty };
                    let fwd_key_for_log = fwd_key.clone();
                    let old_val = state.store_fwd_map.insert(fwd_key, (*val, version));
                    state
                        .store_fwd_rollback_log
                        .push((fwd_key_for_log, old_val));
                }
            }
            // Store has no dest, so no VN to assign. Just keep the instruction.
            new_instructions.push(inst);
            continue;
        }

        match state.make_expr_key(&inst) {
            Some((expr_key, dest)) => {
                let is_load = expr_key.is_load();

                // For loads, first try store-to-load forwarding before load CSE.
                // This catches the pattern: store V -> *P; load *P -> replace with V.
                if is_load {
                    if let ExprKey::Load {
                        ptr: ref ptr_vn,
                        ty,
                    } = expr_key
                    {
                        let fwd_key = StoreFwdKey {
                            ptr_vn: ptr_vn.clone(),
                            ty,
                        };
                        if let Some((stored_op, version)) = state.store_fwd_map.get(&fwd_key) {
                            if std::env::var_os("CCC_DEBUG_GVN").is_some() {
                                eprintln!(
                                    "[GVNDBG] fwd cand ptr_vn={:?} valid={}",
                                    ptr_vn,
                                    state.entry_valid_for(ptr_vn, *version)
                                );
                            }
                            if state.entry_valid_for(ptr_vn, *version) {
                                let stored_op = *stored_op;
                                if std::env::var_os("CCC_DEBUG_GVN").is_some() {
                                    eprintln!(
                                        "[GVNDBG] FORWARD store->load dest={} stored={:?}",
                                        dest.0, stored_op
                                    );
                                }
                                // Forward the stored value to the load destination.
                                // Assign the dest a VN matching the stored value.
                                let dest_idx = dest.0 as usize;
                                let forwarded_vn = match &stored_op {
                                    Operand::Value(v) => {
                                        let idx = v.0 as usize;
                                        if idx < state.value_numbers.len()
                                            && state.value_numbers[idx] != u32::MAX
                                        {
                                            state.value_numbers[idx]
                                        } else {
                                            state.fresh_vn()
                                        }
                                    }
                                    _ => state.fresh_vn(),
                                };
                                if dest_idx < state.value_numbers.len() {
                                    let old_vn = state.value_numbers[dest_idx];
                                    state.vn_log.push((dest_idx, old_vn));
                                    state.value_numbers[dest_idx] = forwarded_vn;
                                }
                                // Also update load CSE map so subsequent loads from the
                                // same pointer can CSE with this load's dest.
                                let version = state.memory_version(ptr_vn);
                                let load_key_for_log = expr_key.clone();
                                let old_load =
                                    state.load_expr_to_value.insert(expr_key, (dest, version));
                                state.load_rollback_log.push((load_key_for_log, old_load));
                                new_instructions.push(Instruction::Copy {
                                    dest,
                                    src: stored_op,
                                });
                                block_defs.insert(dest.0);
                                eliminated += 1;
                                continue;
                            }
                        }
                    }
                }

                // Look up: check pure expr map, or load map with generation check
                let existing = if is_load {
                    let ptr_vn = match &expr_key {
                        ExprKey::Load { ptr, .. } => ptr.clone(),
                        _ => unreachable!(),
                    };
                    state
                        .load_expr_to_value
                        .get(&expr_key)
                        .and_then(|&(val, version)| {
                            if state.entry_valid_for(&ptr_vn, version) {
                                Some(val)
                            } else {
                                None
                            }
                        })
                } else {
                    state.expr_to_value.get(&expr_key).copied()
                };

                // Only CSE within the same block to avoid cross-block Copy issues.
                // Cross-block CSE creates Copies whose source values may have
                // their registers reused by the allocator before the Copy executes.
                let same_block_existing = existing.filter(|ev| block_defs.contains(&ev.0));
                if let Some(existing_value) = same_block_existing {
                    if std::env::var_os("CCC_DEBUG_GVN").is_some() {
                        eprintln!("[GVNDBG] CSE load dest={} <- {}", dest.0, existing_value.0);
                    }
                    let idx = existing_value.0 as usize;
                    let existing_vn = if idx < state.value_numbers.len()
                        && state.value_numbers[idx] != u32::MAX
                    {
                        state.value_numbers[idx]
                    } else {
                        state.fresh_vn()
                    };
                    let dest_idx = dest.0 as usize;
                    if dest_idx < state.value_numbers.len() {
                        let old_vn = state.value_numbers[dest_idx];
                        state.vn_log.push((dest_idx, old_vn));
                        state.value_numbers[dest_idx] = existing_vn;
                    }
                    new_instructions.push(Instruction::Copy {
                        dest,
                        src: Operand::Value(existing_value),
                    });
                    block_defs.insert(dest.0);
                    eliminated += 1;
                } else {
                    // New expression - assign value number and record it
                    let vn = state.fresh_vn();
                    let dest_idx = dest.0 as usize;
                    if dest_idx < state.value_numbers.len() {
                        let old_vn = state.value_numbers[dest_idx];
                        state.vn_log.push((dest_idx, old_vn));
                        state.value_numbers[dest_idx] = vn;
                    }
                    // Record symbol identity for GlobalAddr and the canonical
                    // address key for GEPs over a symbol base, so that memory
                    // keys (load CSE / store forwarding) unify across
                    // deliberately site-local GlobalAddr duplicates (OP-12).
                    match &inst {
                        Instruction::GlobalAddr { name, .. } => {
                            state.record_symbol_vn(name, vn);
                        }
                        Instruction::GetElementPtr {
                            base, offset, ty, ..
                        } => {
                            state.record_gep_addr_key(base, offset, *ty, vn);
                        }
                        _ => {}
                    }
                    // Record in appropriate map with rollback
                    if is_load {
                        let ptr_vn = match &expr_key {
                            ExprKey::Load { ptr, .. } => ptr.clone(),
                            _ => unreachable!(),
                        };
                        let version = state.memory_version(&ptr_vn);
                        let key_for_log = expr_key.clone();
                        let old_val = state.load_expr_to_value.insert(expr_key, (dest, version));
                        state.load_rollback_log.push((key_for_log, old_val));
                    } else {
                        let key_for_log = expr_key.clone();
                        let old_val = state.expr_to_value.insert(expr_key, dest);
                        state.rollback_log.push((key_for_log, old_val));
                    }
                    state.track_ptr_base(&inst);
                    new_instructions.push(inst);
                    block_defs.insert(dest.0);
                }
            }
            None => {
                // Not a numberable expression (store, call, alloca, etc.)
                if let Instruction::GetElementPtr {
                    dest, base, offset, ..
                } = &inst
                {
                    // GEP offsets are already byte offsets, so pointee metadata
                    // does not affect the resulting address. Canonicalize only
                    // base+offset for memory aliasing; retain the actual GEP.
                    let key = (
                        state.operand_to_vn(&Operand::Value(*base)),
                        state.operand_to_vn(offset),
                    );
                    let vn = match state.gep_value_numbers.get(&key).copied() {
                        Some(vn) => vn,
                        None => {
                            let vn = state.fresh_vn();
                            state.gep_value_numbers.insert(key, vn);
                            vn
                        }
                    };
                    let idx = dest.0 as usize;
                    if idx < state.value_numbers.len() {
                        let old = state.value_numbers[idx];
                        state.vn_log.push((idx, old));
                        state.value_numbers[idx] = vn;
                    }
                    // Canonical address key for symbol-based GEPs that reach
                    // the non-numberable path (same rationale as above).
                    state.record_gep_addr_key(base, offset, inst_ty_of(&inst), vn);
                } else if let Instruction::Copy {
                    dest,
                    src: Operand::Value(src),
                } = &inst
                {
                    // Copies are value-numbering TRANSPARENT: the dest
                    // inherits the source's VN. GEP/load/store-forward keys
                    // computed through a Copy then match the keys computed
                    // from the original value. This is what lets a
                    // store-to-load forward fire when one access goes through
                    // the original GlobalAddr and the other through a
                    // GlobalAddr-CSE'd Copy of it (g1[i]=5 ... load g1[i]
                    // previously missed: the Copy's fresh VN made the two GEP
                    // keys unequal, so the forward candidate lookup never
                    // hit). CSE-replacement Copies already share the
                    // canonical VN with their source; this extends the same
                    // invariant to surviving Copies.
                    let src_vn = state.operand_to_vn(&Operand::Value(*src));
                    if let VNOperand::ValueNum(vn) = src_vn {
                        let idx = dest.0 as usize;
                        if idx >= state.value_numbers.len() {
                            state.value_numbers.resize(idx + 1, u32::MAX);
                        }
                        let old = state.value_numbers[idx];
                        state.vn_log.push((idx, old));
                        state.value_numbers[idx] = vn;
                    } else {
                        // Copy of a constant: nothing to inherit.
                        state.assign_fresh_vn(*dest);
                    }
                } else if let Some(dest) = inst.dest() {
                    state.assign_fresh_vn(dest);
                }
                if let Some(dest) = inst.dest() {
                    block_defs.insert(dest.0);
                }
                state.track_ptr_base(&inst);
                new_instructions.push(inst);
            }
        }
    }

    func.blocks[block_idx].instructions = new_instructions;
    func.blocks[block_idx].source_spans = new_spans;
    eliminated
}

/// Canonicalize operand order for commutative operations.
/// Ensures (a + b) and (b + a) hash to the same key.
fn canonical_order(lhs: VNOperand, rhs: VNOperand) -> (VNOperand, VNOperand) {
    if should_swap(&lhs, &rhs) {
        (rhs, lhs)
    } else {
        (lhs, rhs)
    }
}

fn should_swap(lhs: &VNOperand, rhs: &VNOperand) -> bool {
    match (lhs, rhs) {
        (VNOperand::ValueNum(_), VNOperand::Const(_)) => true,
        (VNOperand::ValueNum(a), VNOperand::ValueNum(b)) => a > b,
        (VNOperand::Const(a), VNOperand::Const(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, CallInfo, IrConst, IrModule, Terminator};

    #[test]
    fn test_commutative_cse() {
        // Test that a + b and b + a are recognized as the same expression
        let block = BasicBlock {
            label: BlockId(0),
            instructions: vec![
                // %0 = add %a, %b
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(1)),
                    ty: IrType::I32,
                },
                // %1 = add %b, %a  (same expression, reversed operands)
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Value(Value(0)),
                    ty: IrType::I32,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        };

        let func = IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::I32,
            blocks: vec![block],
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id: 4,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
        };

        let mut module = IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            char16_string_literals: vec![],
            symver_directives: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
        };

        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1);

        // Second instruction should be a Copy
        match &module.functions[0].blocks[0].instructions[1] {
            Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } => {
                assert_eq!(dest.0, 3);
                assert_eq!(v.0, 2);
            }
            other => panic!("Expected Copy instruction, got {:?}", other),
        }
    }

    #[test]
    fn test_non_commutative_not_cse() {
        // Test that a - b and b - a are NOT treated as the same
        let func = IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::I32,
            blocks: vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Sub,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Value(Value(1)),
                        ty: IrType::I32,
                    },
                    Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Sub,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Value(Value(0)),
                        ty: IrType::I32,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                source_spans: Vec::new(),
            }],
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id: 4,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            is_gnu_inline_def: false,
            ret_is_f128_sse: false,
            loop_promoted_f64_values: Vec::new(),
        };

        let mut module = IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            char16_string_literals: vec![],
            symver_directives: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
        };

        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 0);
    }

    #[test]
    fn test_constant_cse() {
        // Two identical constant expressions should be CSE'd
        let func = IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::I32,
            blocks: vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::BinOp {
                        dest: Value(0),
                        op: IrBinOp::Add,
                        lhs: Operand::Const(IrConst::I32(3)),
                        rhs: Operand::Const(IrConst::I32(4)),
                        ty: IrType::I32,
                    },
                    Instruction::BinOp {
                        dest: Value(1),
                        op: IrBinOp::Add,
                        lhs: Operand::Const(IrConst::I32(3)),
                        rhs: Operand::Const(IrConst::I32(4)),
                        ty: IrType::I32,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
                source_spans: Vec::new(),
            }],
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id: 2,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            is_gnu_inline_def: false,
            ret_is_f128_sse: false,
            loop_promoted_f64_values: Vec::new(),
        };

        let mut module = IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            char16_string_literals: vec![],
            symver_directives: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
        };

        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1);
    }

    #[test]
    fn test_is_commutative() {
        assert!(IrBinOp::Add.is_commutative());
        assert!(IrBinOp::Mul.is_commutative());
        assert!(!IrBinOp::Sub.is_commutative());
        assert!(!IrBinOp::SDiv.is_commutative());
    }

    #[test]
    fn test_cast_cse() {
        // Two identical casts should be CSE'd
        let func = IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::I64,
            blocks: vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Cast {
                        dest: Value(1),
                        src: Operand::Value(Value(0)),
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                    },
                    Instruction::Cast {
                        dest: Value(2),
                        src: Operand::Value(Value(0)),
                        from_ty: IrType::I32,
                        to_ty: IrType::I64,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                source_spans: Vec::new(),
            }],
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id: 3,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            is_gnu_inline_def: false,
            ret_is_f128_sse: false,
            loop_promoted_f64_values: Vec::new(),
        };

        let mut module = IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            char16_string_literals: vec![],
            symver_directives: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
        };

        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1);

        match &module.functions[0].blocks[0].instructions[1] {
            Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } => {
                assert_eq!(dest.0, 2);
                assert_eq!(v.0, 1);
            }
            other => panic!("Expected Copy instruction, got {:?}", other),
        }
    }

    #[test]
    fn test_gep_cse() {
        // GEP CSE is currently disabled (creates Copy chains with stale registers).
        // This test verifies GEPs are NOT CSE'd.
        let func = IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::Ptr,
            blocks: vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::GetElementPtr {
                        dest: Value(2),
                        base: Value(0),
                        offset: Operand::Value(Value(1)),
                        ty: IrType::Ptr,
                    },
                    Instruction::GetElementPtr {
                        dest: Value(3),
                        base: Value(0),
                        offset: Operand::Value(Value(1)),
                        ty: IrType::Ptr,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                source_spans: Vec::new(),
            }],
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id: 4,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            is_gnu_inline_def: false,
            ret_is_f128_sse: false,
            loop_promoted_f64_values: Vec::new(),
        };

        let mut module = IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            char16_string_literals: vec![],
            symver_directives: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
        };

        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1); // two identical GEPs CSE'd into one + Copy
    }

    #[test]
    fn test_cross_block_cse() {
        // Cross-block CSE is currently restricted to same-block only.
        // This test verifies cross-block expressions are NOT CSE'd.
        // CFG: block0 -> block1 (block0 dominates block1)
        let func = IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::I32,
            blocks: vec![
                BasicBlock {
                    label: BlockId(0),
                    instructions: vec![Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Value(Value(1)),
                        ty: IrType::I32,
                    }],
                    terminator: Terminator::Branch(BlockId(1)),
                    source_spans: Vec::new(),
                },
                BasicBlock {
                    label: BlockId(1),
                    instructions: vec![
                        // Same expression as in block0 - should be CSE'd
                        Instruction::BinOp {
                            dest: Value(3),
                            op: IrBinOp::Add,
                            lhs: Operand::Value(Value(0)),
                            rhs: Operand::Value(Value(1)),
                            ty: IrType::I32,
                        },
                    ],
                    terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                    source_spans: Vec::new(),
                },
            ],
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id: 4,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            is_gnu_inline_def: false,
            ret_is_f128_sse: false,
            loop_promoted_f64_values: Vec::new(),
        };

        let mut module = IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            char16_string_literals: vec![],
            symver_directives: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
        };

        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 0); // Cross-block CSE disabled (same-block only)

        // The expression in block1 should NOT be replaced (cross-block CSE disabled)
        match &module.functions[0].blocks[1].instructions[0] {
            Instruction::BinOp { dest, .. } => {
                assert_eq!(dest.0, 3); // Original BinOp preserved
            }
            other => panic!("Expected BinOp (cross-block CSE disabled), got {:?}", other),
        }
    }

    #[test]
    fn test_diamond_no_cse_between_branches() {
        // Diamond CFG: block0 -> {block1, block2} -> block3
        // Expressions in block1 and block2 should NOT be CSE'd with each other,
        // since neither dominates the other.
        let func = IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::I32,
            blocks: vec![
                // block0: entry, branches to block1 or block2
                BasicBlock {
                    label: BlockId(0),
                    instructions: vec![],
                    terminator: Terminator::CondBranch {
                        cond: Operand::Value(Value(0)),
                        true_label: BlockId(1),
                        false_label: BlockId(2),
                    },
                    source_spans: Vec::new(),
                },
                // block1: compute add (only reached via true branch)
                BasicBlock {
                    label: BlockId(1),
                    instructions: vec![Instruction::BinOp {
                        dest: Value(2),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    }],
                    terminator: Terminator::Branch(BlockId(3)),
                    source_spans: Vec::new(),
                },
                // block2: compute same add (only reached via false branch)
                BasicBlock {
                    label: BlockId(2),
                    instructions: vec![Instruction::BinOp {
                        dest: Value(3),
                        op: IrBinOp::Add,
                        lhs: Operand::Value(Value(0)),
                        rhs: Operand::Const(IrConst::I32(1)),
                        ty: IrType::I32,
                    }],
                    terminator: Terminator::Branch(BlockId(3)),
                    source_spans: Vec::new(),
                },
                // block3: merge
                BasicBlock {
                    label: BlockId(3),
                    instructions: vec![],
                    terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
                    source_spans: Vec::new(),
                },
            ],
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id: 4,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            is_gnu_inline_def: false,
            ret_is_f128_sse: false,
            loop_promoted_f64_values: Vec::new(),
        };

        let mut module = IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            char16_string_literals: vec![],
            symver_directives: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
        };

        let eliminated = module.for_each_function(run_gvn_function);
        // Neither branch dominates the other, so NO CSE should happen
        assert_eq!(eliminated, 0);

        // Both blocks should still have their original BinOp instructions
        assert!(matches!(
            &module.functions[0].blocks[1].instructions[0],
            Instruction::BinOp {
                op: IrBinOp::Add,
                ..
            }
        ));
        assert!(matches!(
            &module.functions[0].blocks[2].instructions[0],
            Instruction::BinOp {
                op: IrBinOp::Add,
                ..
            }
        ));
    }

    /// Helper to create a minimal IrFunction with given blocks.
    fn make_func(blocks: Vec<BasicBlock>, next_value_id: u32) -> IrFunction {
        IrFunction {
            name: "test".to_string(),
            params: vec![],
            return_type: IrType::I32,
            blocks,
            is_variadic: false,
            is_fastcall: false,
            is_naked: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            is_noinline: false,
            is_declaration: false,
            next_value_id,
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
        }
    }

    fn make_module(func: IrFunction) -> IrModule {
        IrModule {
            functions: vec![func],
            extern_function_symbols: crate::common::fx_hash::FxHashSet::default(),
            globals: vec![],
            string_literals: vec![],
            wide_string_literals: vec![],
            constructors: vec![],
            destructors: vec![],
            aliases: vec![],
            toplevel_asm: vec![],
            symbol_attrs: vec![],
            asm_labels: crate::common::fx_hash::FxHashMap::default(),
            char16_string_literals: vec![],
            symver_directives: vec![],
        }
    }

    #[test]
    fn test_load_cse_same_block() {
        // Two loads from the same pointer in the same block should be CSE'd
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                source_spans: Vec::new(),
            }],
            3,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1);

        // Second load should be replaced with Copy
        match &module.functions[0].blocks[0].instructions[1] {
            Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } => {
                assert_eq!(dest.0, 2);
                assert_eq!(v.0, 1);
            }
            other => panic!("Expected Copy, got {:?}", other),
        }
    }

    #[test]
    fn test_load_cse_invalidated_by_store() {
        // A store between two loads should prevent CSE
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(42)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                source_spans: Vec::new(),
            }],
            3,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        // The store invalidates the first load's CSE entry, but store-to-load
        // forwarding can replace the second load with the stored value (42).
        assert_eq!(eliminated, 1);
    }

    #[test]
    fn test_load_cse_invalidated_by_call() {
        // A call between two loads should prevent CSE
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Call {
                        func: "side_effect".to_string(),
                        info: CallInfo {
                            dest: Some(Value(2)),
                            args: vec![],
                            arg_types: vec![],
                            return_type: IrType::Void,
                            is_variadic: false,
                            num_fixed_args: 0,
                            struct_arg_sizes: vec![],
                            struct_arg_aligns: vec![],
                            struct_arg_classes: vec![],
                            struct_arg_riscv_float_classes: Vec::new(),
                            struct_arg_is_f128_sse: Vec::new(),
                            is_sret: false,
                            is_fastcall: false,
                            ret_eightbyte_classes: Vec::new(),
                            ret_is_f128_sse: false,
                        },
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(3),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                source_spans: Vec::new(),
            }],
            4,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 0); // No CSE: call may modify memory
    }

    #[test]
    fn test_load_cse_across_dominating_block() {
        // Cross-block Load CSE is currently disabled (same-block CSE only).
        // Load in block1 should NOT be CSE'd with load in block0.
        let func = make_func(
            vec![
                BasicBlock {
                    label: BlockId(0),
                    instructions: vec![Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    }],
                    terminator: Terminator::Branch(BlockId(1)),
                    source_spans: Vec::new(),
                },
                BasicBlock {
                    label: BlockId(1),
                    instructions: vec![Instruction::Load {
                        volatile: false,
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    }],
                    terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                    source_spans: Vec::new(),
                },
            ],
            3,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 0); // Cross-block Load CSE disabled

        // Load should be preserved (not replaced with Copy)
        match &module.functions[0].blocks[1].instructions[0] {
            Instruction::Load { dest, .. } => {
                assert_eq!(dest.0, 2);
            }
            other => panic!("Expected Load (cross-block CSE disabled), got {:?}", other),
        }
    }

    #[test]
    fn test_load_cse_invalidated_at_merge_point() {
        // Diamond CFG: block0 -> {block1, block2} -> block3
        // block1 stores to memory, so Load CSE should be invalidated at block3
        let func = make_func(
            vec![
                // block0: entry, load and branch
                BasicBlock {
                    label: BlockId(0),
                    instructions: vec![Instruction::Load {
                        volatile: false,
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    }],
                    terminator: Terminator::CondBranch {
                        cond: Operand::Value(Value(1)),
                        true_label: BlockId(1),
                        false_label: BlockId(2),
                    },
                    source_spans: Vec::new(),
                },
                // block1: stores to memory
                BasicBlock {
                    label: BlockId(1),
                    instructions: vec![Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(42)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    }],
                    terminator: Terminator::Branch(BlockId(3)),
                    source_spans: Vec::new(),
                },
                // block2: no memory modification
                BasicBlock {
                    label: BlockId(2),
                    instructions: vec![],
                    terminator: Terminator::Branch(BlockId(3)),
                    source_spans: Vec::new(),
                },
                // block3: merge point - loads from same pointer
                BasicBlock {
                    label: BlockId(3),
                    instructions: vec![Instruction::Load {
                        volatile: false,
                        dest: Value(3),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    }],
                    terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                    source_spans: Vec::new(),
                },
            ],
            4,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        // Load in block3 should NOT be CSE'd because block3 is a merge point
        // and block1 (a predecessor) stores to memory.
        assert_eq!(eliminated, 0);

        // block3's load should remain as-is
        assert!(matches!(
            &module.functions[0].blocks[3].instructions[0],
            Instruction::Load { .. }
        ));
    }

    #[test]
    fn test_store_to_load_forwarding_same_block() {
        // store 42 -> *ptr; load *ptr => should be forwarded to Copy(42)
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(42)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
                source_spans: Vec::new(),
            }],
            2,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1);

        // The load should be replaced with a Copy of the stored constant
        match &module.functions[0].blocks[0].instructions[1] {
            Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::I32(42)),
            } => {
                assert_eq!(dest.0, 1);
            }
            other => panic!("Expected Copy of constant 42, got {:?}", other),
        }
    }

    #[test]
    fn test_store_to_load_forwarding_value() {
        // store %v -> *ptr; load *ptr => should be forwarded to Copy(%v)
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Value(Value(1)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                source_spans: Vec::new(),
            }],
            3,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1);

        match &module.functions[0].blocks[0].instructions[1] {
            Instruction::Copy {
                dest,
                src: Operand::Value(v),
            } => {
                assert_eq!(dest.0, 2);
                assert_eq!(v.0, 1);
            }
            other => panic!("Expected Copy of Value(1), got {:?}", other),
        }
    }

    #[test]
    fn test_store_to_load_forwarding_invalidated_by_call() {
        // store 42 -> *ptr; call foo(); load *ptr => NOT forwarded (call may modify memory)
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(42)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Call {
                        func: "foo".to_string(),
                        info: CallInfo {
                            dest: Some(Value(1)),
                            args: vec![],
                            arg_types: vec![],
                            return_type: IrType::Void,
                            is_variadic: false,
                            num_fixed_args: 0,
                            struct_arg_sizes: vec![],
                            struct_arg_aligns: vec![],
                            struct_arg_classes: vec![],
                            struct_arg_riscv_float_classes: Vec::new(),
                            struct_arg_is_f128_sse: Vec::new(),
                            is_sret: false,
                            is_fastcall: false,
                            ret_eightbyte_classes: Vec::new(),
                            ret_is_f128_sse: false,
                        },
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                source_spans: Vec::new(),
            }],
            3,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 0); // No forwarding: call invalidates the store
    }

    #[test]
    fn test_store_to_load_forwarding_different_store_invalidates() {
        // store 42 -> *ptr_a; store 99 -> *ptr_b; load *ptr_a
        // => NOT forwarded because the second store (to any address) invalidates all
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(42)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(99)),
                        ptr: Value(1),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(2),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(2)))),
                source_spans: Vec::new(),
            }],
            3,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        // The second store bumps the generation, invalidating the first store's entry.
        // However, the second store also records *ptr_b -> 99 at the new generation.
        // The load from *ptr_a should NOT be forwarded because *ptr_a != *ptr_b.
        assert_eq!(eliminated, 0);
    }

    #[test]
    fn test_store_to_load_forwarding_same_ptr_overwrite() {
        // store 42 -> *ptr; store 99 -> *ptr; load *ptr => forwarded to 99
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(42)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Store {
                        volatile: false,
                        val: Operand::Const(IrConst::I32(99)),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                    Instruction::Load {
                        volatile: false,
                        dest: Value(1),
                        ptr: Value(0),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(1)))),
                source_spans: Vec::new(),
            }],
            2,
        );

        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 1);

        // Should forward the SECOND store's value (99), not the first (42)
        match &module.functions[0].blocks[0].instructions[2] {
            Instruction::Copy {
                dest,
                src: Operand::Const(IrConst::I32(99)),
            } => {
                assert_eq!(dest.0, 1);
            }
            other => panic!("Expected Copy of constant 99, got {:?}", other),
        }
    }

    #[test]
    fn gvn_skips_functions_with_128bit_ops() {
        // Regression: GVN CSE of 128-bit vector operations produced wrong
        // results for SSE/PCLMUL CRC (gzip memcpy-abuse). Functions that use
        // 128-bit typed values must skip GVN because the backend holds such
        // values in XMM register pairs and GVN-inserted Copies can cross
        // pair boundaries.
        let block = BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Xor,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(0)),
                    ty: IrType::I128,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Xor,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(0)),
                    ty: IrType::I128,
                },
            ],
            terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
            source_spans: Vec::new(),
        };
        let mut module = make_module(make_func(vec![block], 4));
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 0);
        assert!(matches!(
            &module.functions[0].blocks[0].instructions[1],
            Instruction::BinOp { dest: Value(3), .. }
        ));
    }

    #[test]
    fn gvn_does_not_cse_mixed_class_global_addr() {
        // v1 is only a load pointer (foldable). v2 is a call arg
        // (must-materialize). Class-blind CSE would pin the RIP-foldable
        // address in a GPR.
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::GlobalAddr {
                        dest: Value(1),
                        name: "g".to_string(),
                    },
                    Instruction::Load {
                        dest: Value(3),
                        ptr: Value(1),
                        ty: IrType::I32,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                    },
                    Instruction::GlobalAddr {
                        dest: Value(2),
                        name: "g".to_string(),
                    },
                    Instruction::Call {
                        func: "use_ptr".to_string(),
                        info: CallInfo {
                            dest: None,
                            args: vec![Operand::Value(Value(2))],
                            arg_types: vec![IrType::Ptr],
                            return_type: IrType::Void,
                            is_variadic: false,
                            num_fixed_args: 1,
                            struct_arg_sizes: vec![],
                            struct_arg_aligns: vec![],
                            struct_arg_classes: vec![],
                            struct_arg_riscv_float_classes: vec![],
                            struct_arg_is_f128_sse: Vec::new(),
                            is_sret: false,
                            is_fastcall: false,
                            ret_eightbyte_classes: vec![],
                            ret_is_f128_sse: false,
                        },
                    },
                ],
                terminator: Terminator::Return(None),
                source_spans: Vec::new(),
            }],
            4,
        );
        let mut module = make_module(func);
        let eliminated = module.for_each_function(run_gvn_function);
        assert_eq!(eliminated, 0);
        assert_eq!(module.functions[0].blocks[0].instructions.len(), 4);
        assert!(matches!(
            &module.functions[0].blocks[0].instructions[2],
            Instruction::GlobalAddr { dest: Value(2), .. }
        ));
    }

    #[test]
    fn gvn_keeps_variable_index_global_addrs_site_local() {
        let func = make_func(
            vec![BasicBlock {
                label: BlockId(0),
                instructions: vec![
                    Instruction::GlobalAddr {
                        dest: Value(1),
                        name: "table".to_string(),
                    },
                    Instruction::GetElementPtr {
                        dest: Value(2),
                        base: Value(1),
                        offset: Operand::Value(Value(9)),
                        ty: IrType::Ptr,
                    },
                    Instruction::GlobalAddr {
                        dest: Value(3),
                        name: "table".to_string(),
                    },
                    Instruction::GetElementPtr {
                        dest: Value(4),
                        base: Value(3),
                        offset: Operand::Value(Value(8)),
                        ty: IrType::Ptr,
                    },
                ],
                terminator: Terminator::Return(None),
                source_spans: Vec::new(),
            }],
            10,
        );
        let mut module = make_module(func);
        let _ = module.for_each_function(run_gvn_function);
        assert_eq!(
            module.functions[0].blocks[0]
                .instructions
                .iter()
                .filter(|i| matches!(i, Instruction::GlobalAddr { .. }))
                .count(),
            2
        );
    }
}

/// The pointee type of a GEP instruction (helper for canonical address keys).
fn inst_ty_of(inst: &Instruction) -> IrType {
    match inst {
        Instruction::GetElementPtr { ty, .. } => *ty,
        _ => IrType::Ptr,
    }
}
