//! Constant-array promotion: fully-constant local arrays → `.rodata` globals.
//!
//! A local array initialized entirely by constants and only ever READ is a
//! C-level constant, but the pipeline still materializes it as an alloca
//! plus one store per element — 48 scalar `movb`s for vecreg_new_ops'
//! three 16-byte arrays, ~100 stores for a dispatch table — where GCC and
//! Clang emit the bytes once in `.rodata` and reference them
//! rip-relatively. The stores survive every pass because each is a
//! legitimate store to a legitimate alloca; nothing before codegen ever
//! asks "is this whole object a constant?".
//!
//! After full unrolling + SCCP + copy propagation, the surviving shape of
//! such an array is exactly:
//!
//! - one `Alloca` of `size` bytes;
//! - stores whose pointers are the alloca itself or a constant-offset GEP
//!   of it, and whose values are all `Const` — with the stored byte ranges
//!   tiling `[0, size)` completely and without overlap;
//! - every other use of the array's ADDRESS either loads from it or passes
//!   it to a direct callee that provably never writes through that
//!   parameter.
//!
//! The transform replaces the alloca with a fresh `is_const` global
//! (`.LCA_n`, following the `.LCVEC` naming convention) carrying the bytes
//! as a `GlobalInit::Array` of `I8`, substitutes a `GlobalAddr` for every
//! address use, and deletes the initialization. The backend's existing
//! global + GEP folding then emits `tbl+N(%rip)` operands.
//!
//! Soundness contract (fail-closed; a candidate failing any check keeps
//! its alloca):
//!
//! 1. **Exact tiling.** The stores' `[offset, offset+width)` ranges must
//!    cover every byte of the object with no gaps (uninitialized bytes
//!    would change semantics) and no overlap (two stores to one byte means
//!    the later value wins — orderable in principle, but the folded single
//!    value would have to be proven; reject instead).
//! 2. **Dominance.** All initialization stores must live in one block that
//!    dominates every block containing an address use — a conditionally
//!    initialized array is not a constant. (A store whose value is only
//!    conditionally *computed* is fine: SCCP has already folded it, or the
//!    value is not a `Const` and the candidate dies on that store.)
//! 3. **Read-only address discipline, by provenance.** The address may
//!    reach a `Load` pointer or a direct-call argument through ANY chain
//!    of address dataflow — constant- or variable-offset GEPs, casts,
//!    copies, phis, selects — and every such mention is resolved through
//!    `pointer_touches`. A `Store` through any pointer that may touch the
//!    object with a non-`Const` value (or at an unresolvable offset)
//!    rejects it: `a[k & 3] = v` is a runtime write and the object is not
//!    a constant. Any mention in a non-whitelisted position — stored to
//!    memory, returned (terminators are audited: `return a;` once left a
//!    dangling reference to the deleted alloca and ICE'd the backend),
//!    passed to an indirect call or an extern, used in arithmetic, fed to
//!    a `Memcpy`, an atomic, or inline asm — rejects the candidate.
//! 4. **Callee write-through proof.** For a direct-callee argument, the
//!    callee's parameter (the `ParamRef` at that index) may only flow into
//!    `Load` pointers, through pointer-to-pointer `Cast`s, `GEP`s of ANY
//!    offset, and `Phi`s of derived pointers (a loop strength-reduced to
//!    pointer induction — the canonical reader shape at `-O2` — merges the
//!    preheader pointer with the backedge GEP; the phi *may* carry our
//!    object on some path, so writes through it still reject). A store
//!    through any of them, a pass-on to another call, or any other
//!    consumption rejects the candidate at the call site. The check is
//!    deliberately non-transitive: a param forwarded to another function's
//!    param is rejected rather than transitively proven.
//! 5. **Volatile stores never participate** (a volatile store is
//!    observable); atomic accesses reject the candidate outright.
//! 6. **Alignment is preserved.** The promoted global keeps the alloca's
//!    alignment (`int[2]` stays 4-byte aligned; `_Alignas` is honored):
//!    `.rodata` placement at align 1 would silently turn every aligned
//!    access unaligned. The backend's `effective_align` still applies the
//!    size ≥ 16 → 16 promotion, matching GCC/Clang.
//!
//! Pass name for `CCC_DISABLE_PASSES`: "constarr".

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::module::{GlobalInit, IrGlobal, IrModule};
use crate::ir::reexports::{Instruction, IrConst, IrFunction, Operand, Terminator, Value};

/// A store into a candidate array: `(byte_offset, width, constant)`.
struct ByteStore {
    offset: i64,
    width: u32,
    value: IrConst,
}

/// The load/store width in bytes of an IR integer type (the only store
/// types this pass consumes).
fn int_width(ty: IrType) -> Option<u32> {
    Some(match ty {
        IrType::I8 | IrType::U8 => 1,
        IrType::I16 | IrType::U16 => 2,
        IrType::I32 | IrType::U32 => 4,
        IrType::I64 | IrType::U64 => 8,
        _ => return None,
    })
}

/// Render a stored constant as `width` little-endian bytes.
fn const_bytes(c: &IrConst, width: u32) -> Option<Vec<u8>> {
    let v: u128 = match c {
        IrConst::I8(x) => *x as u128,
        IrConst::I16(x) => *x as u128,
        IrConst::I32(x) => *x as u128,
        IrConst::I64(x) => *x as u128,
        IrConst::I128(x) => (*x as u128) & (u128::MAX >> (128 - width * 8).min(127)),
        _ => return None,
    };
    let mut out = Vec::with_capacity(width as usize);
    for i in 0..width {
        out.push(((v >> (i * 8)) & 0xff) as u8);
    }
    Some(out)
}

/// Constant-offset GEP view: dest -> (root base, total offset). Chains
/// collapse iteratively.
fn gep_roots(func: &IrFunction) -> FxHashMap<u32, (u32, i64)> {
    let mut map: FxHashMap<u32, (u32, i64)> = FxHashMap::default();
    // Iterate to a fixed point: a GEP of a GEP resolves once the inner one
    // is in the map (the scan order is not guaranteed).
    loop {
        let mut grew = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::GetElementPtr {
                    dest, base, offset, ..
                } = inst
                {
                    if map.contains_key(&dest.0) {
                        continue;
                    }
                    if let Operand::Const(c) = offset {
                        if let Some(off) = c.to_i64() {
                            let root = match map.get(&base.0) {
                                Some(&(b, o)) => (b, o.saturating_add(off)),
                                None => (base.0, off),
                            };
                            map.insert(dest.0, root);
                            grew = true;
                        }
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    map
}

/// Resolve a pointer value to `(root alloca, byte offset)` when it is the
/// alloca itself or a constant-offset GEP chain above it.
fn addr_of(v: u32, alloca: u32, geps: &FxHashMap<u32, (u32, i64)>) -> Option<i64> {
    if v == alloca {
        return Some(0);
    }
    match geps.get(&v) {
        Some(&(b, off)) if b == alloca => Some(off),
        _ => None,
    }
}

/// The set of candidate allocas a value may point into, resolved through
/// ANY chain of address dataflow — constant- **or variable-offset** GEPs,
/// pointer casts, copies, and phis/selects merging derived pointers (a
/// merge with an unrelated pointer still *may* carry ours on some path,
/// which is exactly the rejection direction needed for stores).
///
/// This is the classification the dereference audit runs on. The original
/// const-offset-only resolution left every variable-offset dereference
/// invisible: `a[k & 3] = v` — a runtime store! — fell through the
/// whitelist's `GEP.base` arm unclassified, so an array whose only other
/// use was a provably read-only callee promoted with the runtime write
/// intact: a store into `.rodata` (SIGSEGV at best, silent corruption in
/// any build that links the section writable). The same invisibility let
/// `Memcopy`/`AtomicStore`/`InlineAsm`-style write positions and
/// `return a;` (a terminator mention) escape the catch-all — the latter
/// left a dangling reference to the deleted alloca and ICE'd the backend.
/// `pointer_touches` closes the whole class: provenance, not spelling.
fn pointer_touches(
    func: &IrFunction,
    allocas: &FxHashMap<u32, (usize, usize)>,
) -> FxHashMap<u32, FxHashSet<u32>> {
    let mut touches: FxHashMap<u32, FxHashSet<u32>> = FxHashMap::default();
    // Seed: each candidate alloca touches itself.
    for &a in allocas.keys() {
        touches.entry(a).or_default().insert(a);
    }
    // Fixed point: sets only grow and are bounded by (values x candidates).
    loop {
        let mut grew = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let (dest, srcs): (u32, Vec<Operand>) = match inst {
                    Instruction::GetElementPtr { dest, base, .. } => {
                        (dest.0, vec![Operand::Value(*base)])
                    }
                    Instruction::Cast { dest, src, .. } => (dest.0, vec![*src]),
                    Instruction::Copy { dest, src } => (dest.0, vec![*src]),
                    Instruction::Phi { dest, incoming, .. } => {
                        (dest.0, incoming.iter().map(|(op, _)| *op).collect())
                    }
                    Instruction::Select {
                        dest,
                        true_val,
                        false_val,
                        ..
                    } => (dest.0, vec![*true_val, *false_val]),
                    _ => continue,
                };
                let mut union: FxHashSet<u32> = FxHashSet::default();
                for op in srcs {
                    if let Operand::Value(v) = op {
                        if let Some(ts) = touches.get(&v.0) {
                            union.extend(ts.iter().copied());
                        }
                    }
                }
                if !union.is_empty() {
                    let entry = touches.entry(dest).or_default();
                    if union.iter().any(|c| !entry.contains(c)) {
                        entry.extend(union);
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    touches
}

/// Does `callee` (a function with a visible body in the module) never write
/// through its `arg_idx`-th parameter? Fail-closed: unknown callees and any
/// non-load consumption reject.
fn callee_readonly_through(all: &[IrFunction], callee: &str, arg_idx: usize) -> bool {
    let Some(f) = all.iter().find(|f| f.name == callee && !f.is_declaration) else {
        return false;
    };
    // The parameter's SSA value: the ParamRef at this index.
    let mut root: Option<u32> = None;
    for block in &f.blocks {
        for inst in &block.instructions {
            if let Instruction::ParamRef {
                dest, param_idx, ..
            } = inst
            {
                if *param_idx == arg_idx {
                    root = Some(dest.0);
                }
            }
        }
    }
    let Some(root) = root else {
        return false;
    };
    // Values derived from the param through pointer casts, const GEPs and
    // PHIS of derived pointers (the vectorizer's scalar remainder leaves a
    // pointer phi merging the preheader pointer and the backedge GEP).
    let mut derived: FxHashSet<u32> = FxHashSet::default();
    derived.insert(root);
    loop {
        let mut grew = false;
        for block in &f.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::GetElementPtr { dest, base, .. } => {
                        // Any GEP — constant OR variable offset — is pure
                        // address arithmetic; the write discipline is
                        // enforced at the USES of the derived pointer. (A
                        // constant-offset restriction here rejected the
                        // canonical `for (i=0;i<n;i++) s += p[i];` reader,
                        // starving the whole transform.)
                        if derived.contains(&base.0) && derived.insert(dest.0) {
                            grew = true;
                        }
                    }
                    Instruction::Cast { dest, src, .. } => {
                        if let Operand::Value(sv) = src {
                            if derived.contains(&sv.0) && derived.insert(dest.0) {
                                grew = true;
                            }
                        }
                    }
                    Instruction::Phi { dest, incoming, .. } => {
                        // A phi merging derived pointer(s) with anything else
                        // is a *conditional* derivation: its dest may name
                        // our array on some paths. Adding it to the derived
                        // set keeps every write through it rejected (the
                        // soundness direction) while reads stay allowed.
                        if incoming.iter().any(
                            |(op, _)| matches!(op, Operand::Value(v) if derived.contains(&v.0)),
                        ) && derived.insert(dest.0)
                        {
                            grew = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        if !grew {
            break;
        }
    }
    // Every use of a derived value must be a memory READ: a `Load` pointer,
    // or a mention inside an intrinsic that does not write through that
    // position. The Intrinsic contract (see `Instruction::may_write_memory`)
    // is that writes go through `dest_ptr` or the `writes_memory_via_args`
    // families — a derived pointer in `args` of an op outside those families
    // is a folded memory operand read (the Paddusb128 family's vector-load
    // folding). Anything else — a Store pointer, a Memcpy, a call, a
    // dest_ptr, an args-writing family — rejects.
    for block in &f.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { ptr, .. } => {
                    if derived.contains(&ptr.0) {
                        continue;
                    }
                    let mut touches = false;
                    inst.for_each_used_value(|id| {
                        if derived.contains(&id) {
                            touches = true;
                        }
                    });
                    if touches {
                        return false;
                    }
                }
                // Derivation steps: the GEP's base mention, the Cast's
                // source mention and the Phi's incoming mentions are how
                // the derived set was built; their DEST is audited wherever
                // it is used.
                Instruction::GetElementPtr { .. }
                | Instruction::Cast { .. }
                | Instruction::Phi { .. } => {}
                Instruction::Intrinsic {
                    dest_ptr, op, args, ..
                } => {
                    if let Some(dp) = dest_ptr {
                        if derived.contains(&dp.0) {
                            return false; // writes through our pointer
                        }
                    }
                    if op.writes_memory_via_args() {
                        for a in args {
                            if let Operand::Value(v) = a {
                                if derived.contains(&v.0) {
                                    return false; // args-writing family
                                }
                            }
                        }
                    }
                    // A read-side args mention of a derived pointer is the
                    // folded-load form: fine. Any OTHER mention inside a
                    // memory-writing intrinsic position is caught above;
                    // non-memory intrinsic uses of pointers do not exist.
                }
                _ => {
                    let mut touches = false;
                    inst.for_each_used_value(|id| {
                        if derived.contains(&id) {
                            touches = true;
                        }
                    });
                    if touches {
                        return false;
                    }
                }
            }
        }
    }
    // Terminators cannot legally mention data pointers in this IR, but be
    // fail-closed anyway.
    for block in &f.blocks {
        let mut touches = false;
        block.terminator.for_each_used_value(|id| {
            if derived.contains(&id) {
                touches = true;
            }
        });
        if touches {
            return false;
        }
    }
    true
}

/// Audit `func` (read-only, against the whole module for callee proofs) and
/// return every promotable candidate: (alloca id, size, align, byte image).
fn audit_candidates(func: &IrFunction, all: &[IrFunction]) -> Vec<(u32, usize, usize, Vec<u8>)> {
    let mut out: Vec<(u32, usize, usize, Vec<u8>)> = Vec::new();
    // ---- collect candidates ------------------------------------------------
    let mut allocas: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest, size, align, ..
            } = inst
            {
                if *size > 0 && *size <= 4096 {
                    allocas.insert(dest.0, (*size, *align));
                }
            }
        }
    }
    if allocas.is_empty() {
        return out;
    }
    let geps = gep_roots(func);
    let touches = pointer_touches(func, &allocas);

    // ---- per-candidate audit ----------------------------------------------
    let mut stores: FxHashMap<u32, Vec<ByteStore>> = FxHashMap::default();
    let mut store_blocks: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    let mut use_blocks: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    let mut bad: FxHashSet<u32> = FxHashSet::default();

    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            // Classification. Every arm FALLS THROUGH to the catch-all
            // below — an early `continue` here is exactly how the escape
            // store (`keep = a`, whose pointer targets a global) skipped
            // its value-side audit once, and a rejected-then-promoted
            // array left an orphaned use that ICE'd the backend.
            //
            // Every arm resolves its pointer through `touches` (provenance
            // through ANY offset/cast/copy/phi/select chain), never through
            // the const-offset-only `geps` view: a runtime store at a
            // variable index must classify as a store, or the object is
            // not a constant.
            match inst {
                Instruction::Store {
                    val,
                    ptr,
                    ty,
                    volatile,
                    ..
                } => {
                    if let Some(ts) = touches.get(&ptr.0) {
                        for &a in ts {
                            if *volatile {
                                bad.insert(a);
                            } else if let Operand::Const(c) = val {
                                if let Some(width) = int_width(*ty) {
                                    match addr_of(ptr.0, a, &geps) {
                                        Some(off) => {
                                            stores.entry(a).or_default().push(ByteStore {
                                                offset: off,
                                                width,
                                                value: c.clone(),
                                            });
                                            store_blocks.entry(a).or_default().push(bi);
                                        }
                                        // A constant value stored through a
                                        // pointer whose offset is not
                                        // resolvable (variable index, or a
                                        // cast/copy/phi-derived base)
                                        // cannot be placed in the image.
                                        None => {
                                            bad.insert(a);
                                        }
                                    }
                                } else {
                                    bad.insert(a);
                                }
                            } else {
                                // A runtime value stored anywhere into the
                                // object: the object is not a constant.
                                // (`a[k & 3] = v` — the variable-offset
                                // GEP is in `touches`, so this now fires.)
                                bad.insert(a);
                            }
                        }
                    }
                }
                Instruction::Load { ptr, volatile, .. } => {
                    if let Some(ts) = touches.get(&ptr.0) {
                        for &a in ts {
                            if *volatile {
                                // A volatile-qualified access keeps the
                                // object in real writable memory; do not
                                // promote what C considers observable.
                                bad.insert(a);
                            } else {
                                use_blocks.entry(a).or_default().push(bi);
                            }
                        }
                    }
                }
                Instruction::GetElementPtr { base, .. } => {
                    if let Some(ts) = touches.get(&base.0) {
                        for &a in ts {
                            use_blocks.entry(a).or_default().push(bi);
                        }
                    }
                }
                Instruction::Call { func: callee, info } => {
                    for (pos, arg) in info.args.iter().enumerate() {
                        if let Operand::Value(v) = arg {
                            if let Some(ts) = touches.get(&v.0) {
                                for &a in ts {
                                    if !callee_readonly_through(all, callee, pos) {
                                        bad.insert(a);
                                    } else {
                                        use_blocks.entry(a).or_default().push(bi);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            // Catch-all: any mention of a candidate's address value in a
            // position the whitelist above does not cover rejects it. The
            // whitelist positions are: Store.ptr, Load.ptr, GEP.base, and
            // direct-Call args (handled above with their own checks).
            let mut ok: FxHashSet<u32> = FxHashSet::default();
            match inst {
                Instruction::Store { ptr, .. } => {
                    ok.insert(ptr.0);
                }
                Instruction::Load { ptr, .. } => {
                    ok.insert(ptr.0);
                }
                Instruction::GetElementPtr { base, .. } => {
                    ok.insert(base.0);
                }
                Instruction::Call { info, .. } => {
                    for arg in &info.args {
                        if let Operand::Value(v) = arg {
                            ok.insert(v.0);
                        }
                    }
                }
                _ => {}
            }
            inst.for_each_used_value(|id| {
                if ok.contains(&id) {
                    return;
                }
                // Provenance, not spelling: any mention of a value that may
                // point into a candidate — through casts, copies, phis,
                // selects, or variable-offset GEPs, none of which the old
                // const-GEP root walk could see — rejects it.
                if let Some(ts) = touches.get(&id) {
                    for &a in ts {
                        bad.insert(a);
                    }
                }
            });
        }
    }
    // Terminators: a data-flow mention of the object's address in a
    // terminator (`return a;`) is an escape — the rewrite would delete the
    // alloca out from under the reference, leaving a dangling use the
    // backend refuses to fabricate (found live: `return a;` ICE'd emit.rs
    // before this scan existed). CondBranch/Switch/IndirectBranch value
    // mentions are equally impossible to honor for an object address.
    for block in &func.blocks {
        let mut hits: Vec<u32> = Vec::new();
        block.terminator.for_each_used_value(|id| {
            if let Some(ts) = touches.get(&id) {
                hits.extend(ts.iter().copied());
            }
        });
        for a in hits {
            bad.insert(a);
        }
    }

    // ---- collect every sound candidate --------------------------------------
    let mut ordered: Vec<u32> = allocas.keys().copied().collect();
    ordered.sort_unstable();
    for a in ordered {
        if bad.contains(&a) {
            continue;
        }
        let size = allocas[&a].0;
        let align = allocas[&a].1.max(1);
        let Some(sts) = stores.get(&a) else {
            continue;
        };
        if sts.is_empty() {
            continue;
        }
        // Exact tiling with no overlap.
        let mut image = vec![0u8; size];
        let mut covered = vec![false; size];
        let mut tiled = true;
        for st in sts {
            let w = st.width as i64;
            if st.offset < 0 || st.offset.saturating_add(w) > size as i64 {
                tiled = false;
                break;
            }
            let Some(bytes) = const_bytes(&st.value, st.width) else {
                tiled = false;
                break;
            };
            for (k, byte) in bytes.into_iter().enumerate() {
                let idx = st.offset as usize + k;
                if covered[idx] {
                    tiled = false;
                    break;
                }
                covered[idx] = true;
                image[idx] = byte;
            }
            if !tiled {
                break;
            }
        }
        if !tiled || covered.iter().any(|c| !*c) {
            continue;
        }
        // Dominance: all stores in one block that dominates every use block.
        let mut site_blocks = store_blocks.get(&a).cloned().unwrap_or_default();
        site_blocks.sort_unstable();
        site_blocks.dedup();
        if site_blocks.len() != 1 {
            continue;
        }
        let store_block = site_blocks[0];
        let dom = dominators(func);
        let mut dominated = true;
        for &ub in use_blocks.get(&a).cloned().unwrap_or_default().as_slice() {
            let mut d = ub;
            let mut found = false;
            let mut guard = 0;
            while d != usize::MAX && guard <= func.blocks.len() {
                if d == store_block {
                    found = true;
                    break;
                }
                let next = dom[d];
                if next == usize::MAX || next == d {
                    break;
                }
                d = next;
                guard += 1;
            }
            if !found {
                dominated = false;
                break;
            }
        }
        if !dominated {
            continue;
        }
        out.push((a, size, align, image));
    }
    out
}

/// Apply one audited candidate's rewrite: substitute the address uses with
/// a fresh `GlobalAddr`, and delete the initialization.
fn apply_one(func: &mut IrFunction, a: u32, global_name: &str) {
    let geps = gep_roots(func);
    let mut fresh = func.next_value_id.max(func.max_value_id() + 1);
    let addr = Value(fresh);
    fresh += 1;
    func.next_value_id = fresh;

    // Delete the initialization stores and the alloca while the alloca id is
    // still meaningful.
    for block in &mut func.blocks {
        block.instructions.retain(|inst| match inst {
            Instruction::Store { ptr, .. } => addr_of(ptr.0, a, &geps).is_none(),
            Instruction::Alloca { dest, .. } => dest.0 != a,
            _ => true,
        });
    }
    // Substitute every remaining address use.
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            match inst {
                Instruction::Load { ptr, .. } => {
                    if ptr.0 == a {
                        *ptr = addr;
                    }
                }
                Instruction::GetElementPtr { base, .. } => {
                    if base.0 == a {
                        *base = addr;
                    }
                }
                Instruction::Call { info, .. } => {
                    for arg in &mut info.args {
                        if let Operand::Value(v) = arg {
                            if v.0 == a {
                                *v = addr;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // Emit the GlobalAddr at the head of the entry block.
    if let Some(entry) = func.blocks.first_mut() {
        entry.instructions.insert(
            0,
            Instruction::GlobalAddr {
                dest: addr,
                name: global_name.to_string(),
            },
        );
    }
}

/// Immediate dominators per block index (usize::MAX for unreachable).
fn dominators(func: &IrFunction) -> Vec<usize> {
    let label_map = crate::ir::analysis::build_label_map(func);
    let (preds, succs) = crate::ir::analysis::build_cfg(func, &label_map);
    crate::ir::analysis::compute_dominators(func.blocks.len(), &preds, &succs)
}

pub(crate) fn run(module: &mut IrModule) -> usize {
    // Phase A (immutable): audit every function against the whole module.
    let mut plans: Vec<(usize, Vec<(u32, usize, usize, Vec<u8>)>)> = Vec::new();
    for (fi, f) in module.functions.iter().enumerate() {
        if f.is_declaration {
            continue;
        }
        let cands = audit_candidates(f, &module.functions);
        if !cands.is_empty() {
            plans.push((fi, cands));
        }
    }
    if plans.is_empty() {
        return 0;
    }
    if std::env::var("CCC_DEBUG_CONSTARR").is_ok() {
        for (fi, cands) in &plans {
            for (a, size, align, _) in cands {
                eprintln!(
                    "[CONSTARR] fn={} (idx {}) alloca v{} size {} align {}",
                    module.functions[*fi].name, fi, a, size, align
                );
            }
        }
    }
    // Phase B (mutable): apply the rewrites and materialize the globals.
    let mut next_label = module
        .globals
        .iter()
        .filter(|g| g.name.starts_with(".LCA_"))
        .count();
    let mut changes = 0usize;
    let mut new_globals: Vec<IrGlobal> = Vec::new();
    for (fi, cands) in plans {
        for (a, size, align, image) in cands {
            let name = format!(".LCA_{}", next_label);
            next_label += 1;
            new_globals.push(IrGlobal {
                name: name.clone(),
                ty: IrType::U8,
                size,
                // The promoted object keeps the alloca's alignment: an
                // int[4] array promised its consumers 4-byte-aligned
                // storage, and .rodata placement at align 1 would silently
                // turn every aligned load unaligned.
                align,
                init: GlobalInit::Array(image.iter().map(|&b| IrConst::I8(b as i8)).collect()),
                is_static: true,
                is_const: true,
                is_extern: false,
                is_common: false,
                section: None,
                is_weak: false,
                visibility: None,
                has_explicit_align: false,
                is_used: true,
                is_thread_local: false,
            });
            apply_one(&mut module.functions[fi], a, &name);
            changes += 1;
        }
    }
    module.globals.extend(new_globals);
    changes
}
