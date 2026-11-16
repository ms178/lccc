//! Collapse a 16-byte aggregate round trip through the integer unit.
//!
//! LCCC has no IR vector type: a `__m128d` / `__m128i` / `__m128` value is a
//! 16-byte aggregate, and an aggregate *value* that crosses a call boundary in
//! a register pair travels as `I128` (see `src/ir/lowering/aggregate_abi.rs`,
//! "packed as I128 in the IR").  When such a function is then inlined -- and
//! every SSE intrinsic wrapper in `include/xmmintrin.h` /
//! `include/emmintrin.h` is `__always_inline__` -- the packing and the
//! unpacking it was written for both survive, and the aggregate round trip
//! becomes eight integer instructions and two GPRs:
//!
//! ```text
//!   %lo  = load %p            (I64)
//!   %hp  = gep %p, 8
//!   %hi  = load %hp           (I64)
//!   %clo = cast %lo -> I128
//!   %chi = cast %hi -> I128
//!   %sh  = shl %chi, 64
//!   %agg = or  %sh, %clo
//!   %dlo = cast shr %agg      -> I64 ; store %q
//!   %shr = lshr %agg, 64
//!   %dhi = cast shr %shr      -> I64 ; store %q + 8
//! ```
//!
//! The net effect of that sequence is a 16-byte copy from `%p` to `%q`, so the
//! integer carrier becomes one `Loaddqu` intrinsic. This is the difference
//! between the stock lowering of `_mm_loadu_pd` and what GCC and Clang emit,
//! and it is why every row of `tests/regression/check_sse2_codegen_matrix.py`
//! used to cost nine more instructions than both oracles: the copy, not the SSE
//! op, was the cost.
//!
//! Correctness is fail-closed and the match is deliberately narrow:
//!
//! * every instruction of the pattern must be in ONE basic block, and each
//!   intermediate value must have exactly one use in the whole function (so
//!   deleting it cannot strand a reader);
//! * the two loads must address `p` and `p + 8` (same base, provable 8-byte
//!   step) and the two stores must address `q` and `q + 8`, so the pattern
//!   really is a whole 16-byte object and never a lane-pair subset;
//! * no memory-clobbering instruction may sit between the loads and the
//!   stores: the rewrite moves the read of `p` later (to the store site), and
//!   a store, call, memcpy, atomic or fence in between could change what `p`
//!   reads;
//! * if `p` and `q` share an object, the ranges must be identical or
//!   non-overlapping. Partial overlap is valid for the original staged
//!   load/store sequence, but the replacement's later 16-byte read could see
//!   the first half-store and produce a different result;
//! * multi-definition value ids (post-phi-coalescing webs) are refused
//!   outright, the same guard `load_forward` and `gvn` use: their content is
//!   position dependent, so a single-use count is not a proof.
//!
//! Pass name for `CCC_DISABLE_PASSES`: "aggcopy16".
//!
//! WHY A PASS AND NOT A BACKEND PEEPHOLE: the pack/unpack pair is separated
//! from the copy it implements by the call/return structure, and it appears in
//! four different backend paths (SSE return, SSE argument, `va_arg`, union
//! member forwarding). Rewriting it once in the IR exposes `Loaddqu` to every
//! backend. On x86 the existing vector-temp forwarding recognizes the paired
//! `Loaddqu`/`Storedqu` operations and folds the copy into the consuming SIMD
//! memory operand; other backends lower the same 16-byte copy operation.

use super::module_symbol_facts::ModuleSymbolFacts;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::provenance::{ChainStep, RootLeaf, chain_step, root_leaf};
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator, Value};

/// Bytes covered by the pattern: two 8-byte halves.
const COPY_BYTES: usize = 16;
const HALF: i64 = 8;

/// The identity of a root object the shared IR provenance rules can prove.
/// Unknown roots are represented by `None`, never by the SSA id of an opaque
/// producer such as a pointer load or a call result.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ObjectRoot {
    Global(String),
    Alloca(u32),
    Param { index: usize, noalias: bool },
}

/// Return the unique definition of `value`. Duplicate definitions make the
/// pointer chain position-dependent, so they cannot support an alias proof.
fn unique_def(func: &IrFunction, value: Value) -> Option<&Instruction> {
    let mut found = None;
    for block in &func.blocks {
        for inst in &block.instructions {
            if inst.dest() == Some(value) {
                if found.is_some() {
                    return None;
                }
                found = Some(inst);
            }
        }
    }
    found
}

/// Trace pointer provenance to a typed object root and, when possible, its
/// constant byte offset. This pass uses the canonical `root_leaf` and
/// `chain_step` rules: GEP/Copy/Ptr-to-Ptr-cast chains are followed, while a
/// load, call, integer-pointer conversion, unknown definition, or cycle fails
/// closed. A dynamic GEP offset retains the root but loses only the offset.
fn root_and_offset(func: &IrFunction, start: Value) -> Option<(ObjectRoot, Option<i64>)> {
    let mut cur = start;
    let mut offset = Some(0i64);
    let mut seen = FxHashSet::default();
    for _ in 0..128 {
        if !seen.insert(cur) {
            return None;
        }
        let inst = unique_def(func, cur)?;
        match root_leaf(inst) {
            RootLeaf::Global(name) => return Some((ObjectRoot::Global(name), offset)),
            RootLeaf::Alloca(id) => return Some((ObjectRoot::Alloca(id), offset)),
            RootLeaf::Param(index) => {
                let noalias = func.params.get(index).is_some_and(|param| param.noalias);
                return Some((ObjectRoot::Param { index, noalias }, offset));
            }
            RootLeaf::NotLeaf => {}
        }
        if let Instruction::GetElementPtr { offset: step, .. } = inst {
            let step = match step {
                Operand::Const(IrConst::I64(bytes)) => Some(*bytes),
                Operand::Const(IrConst::I32(bytes)) => Some(i64::from(*bytes)),
                _ => None,
            };
            offset = match (offset, step) {
                (Some(total), Some(step)) => total.checked_add(step),
                _ => None,
            };
        }
        match chain_step(inst) {
            ChainStep::Follow(next) => cur = next,
            ChainStep::Stop => return None,
        }
    }
    None
}

fn same_root(a: &ObjectRoot, b: &ObjectRoot) -> bool {
    match (a, b) {
        (ObjectRoot::Global(a), ObjectRoot::Global(b)) => a == b,
        (ObjectRoot::Alloca(a), ObjectRoot::Alloca(b)) => a == b,
        (ObjectRoot::Param { index: a, .. }, ObjectRoot::Param { index: b, .. }) => a == b,
        _ => false,
    }
}

/// Object/restrict rules for this copy proof: distinct current-frame stack
/// homes cannot alias; a current-frame alloca cannot alias an incoming
/// parameter; a global may alias an ordinary parameter; and two parameters
/// need a restrict/noalias contract. Distinct global names are disjoint when
/// `ModuleSymbolFacts::globals_distinct` proves it (one strong, locally binding
/// definition and no alias/asm-label redirect). Unknown roots never reach this
/// function because `root_and_offset` returns `None` for them.
fn roots_proven_distinct(a: &ObjectRoot, b: &ObjectRoot, facts: &ModuleSymbolFacts) -> bool {
    match (a, b) {
        (ObjectRoot::Alloca(a), ObjectRoot::Alloca(b)) => a != b,
        (ObjectRoot::Global(a), ObjectRoot::Global(b)) => facts.globals_distinct(a, b),
        (ObjectRoot::Alloca(_), ObjectRoot::Global(_))
        | (ObjectRoot::Global(_), ObjectRoot::Alloca(_))
        | (ObjectRoot::Alloca(_), ObjectRoot::Param { .. })
        | (ObjectRoot::Param { .. }, ObjectRoot::Alloca(_)) => true,
        (ObjectRoot::Global(_), ObjectRoot::Param { noalias, .. })
        | (ObjectRoot::Param { noalias, .. }, ObjectRoot::Global(_)) => *noalias,
        (
            ObjectRoot::Param {
                index: a_index,
                noalias: a_noalias,
            },
            ObjectRoot::Param {
                index: b_index,
                noalias: b_noalias,
            },
        ) => a_index != b_index && (*a_noalias || *b_noalias),
    }
}

/// Follow only provenance-preserving edges from `start` until `target` is
/// reached, accumulating the constant byte delta. This relative proof remains
/// available when both pointers share a dynamic GEP: the dynamic offset is
/// already embodied in the common SSA value and cancels out.
fn offset_to_ancestor(func: &IrFunction, start: Value, target: Value) -> Option<i64> {
    let mut current = start;
    let mut offset = 0i64;
    let mut seen = FxHashSet::default();
    for _ in 0..128 {
        if current == target {
            return Some(offset);
        }
        if !seen.insert(current) {
            return None;
        }
        let inst = unique_def(func, current)?;
        if let Instruction::GetElementPtr { offset: step, .. } = inst {
            let step = match step {
                Operand::Const(IrConst::I64(bytes)) => *bytes,
                Operand::Const(IrConst::I32(bytes)) => i64::from(*bytes),
                _ => return None,
            };
            offset = offset.checked_add(step)?;
        }
        match chain_step(inst) {
            ChainStep::Follow(next) => current = next,
            ChainStep::Stop => return None,
        }
    }
    None
}

/// `a - b` in bytes when it is provable: equal pointers are 0; a pointer
/// directly derived from the other has a relative offset even when the shared
/// base has a dynamic GEP; otherwise two chains rooted at the same object must
/// have known absolute offsets. `None` means "cannot prove" -- callers fail
/// closed.
fn offset_from(func: &IrFunction, a: Value, b: Value) -> Option<i64> {
    if a == b {
        return Some(0);
    }
    if let Some(offset) = offset_to_ancestor(func, a, b) {
        return Some(offset);
    }
    if let Some(offset) = offset_to_ancestor(func, b, a) {
        return offset.checked_neg();
    }
    let (a_root, a_offset) = root_and_offset(func, a)?;
    let (b_root, b_offset) = root_and_offset(func, b)?;
    if !same_root(&a_root, &b_root) {
        return None;
    }
    a_offset?.checked_sub(b_offset?)
}

fn is_cst(v: &Operand, want: i64) -> bool {
    match v {
        Operand::Const(IrConst::I64(x)) => *x == want,
        Operand::Const(IrConst::I32(x)) => i64::from(*x) == want,
        _ => false,
    }
}

/// Memory-writing operations and ordering/observation barriers across which
/// this pass may not move the source read. `may_write_memory` is exhaustive;
/// the extra cases cover operations that order or expose memory without
/// writing bytes (fences, atomic/volatile loads, and stack restoration).
fn memory_motion_barrier(inst: &Instruction) -> bool {
    inst.may_write_memory()
        || matches!(
            inst,
            Instruction::Load { volatile: true, .. }
                | Instruction::AtomicLoad { .. }
                | Instruction::AtomicStore { .. }
                | Instruction::AtomicRmw { .. }
                | Instruction::AtomicInc { .. }
                | Instruction::AtomicCmpxchg { .. }
                | Instruction::Fence { .. }
                | Instruction::StackRestore { .. }
                | Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::Lfence
                        | crate::ir::intrinsics::IntrinsicOp::Mfence
                        | crate::ir::intrinsics::IntrinsicOp::Sfence,
                    ..
                }
        )
}

/// Immutable facts about one candidate, resolved before any mutation.
struct Candidate {
    block: usize,
    drop: Vec<usize>,
    insert_at: usize,
    dest_ptr: Value,
    src_ptr: Value,
}

/// The alloca a store destination provably belongs to, when it is a stack
/// home. Unknown or non-stack destinations remain opaque to the clobber scan.
fn store_root(func: &IrFunction, ptr: Value) -> Option<u32> {
    match root_and_offset(func, ptr)?.0 {
        ObjectRoot::Alloca(root) => Some(root),
        ObjectRoot::Global(_) | ObjectRoot::Param { .. } => None,
    }
}

/// Whether a 16-byte copy from `src` to `dst` can be proven not to overlap
/// partially. Same-pointer copies are exact; same-object pointers are safe
/// only at offset zero or at least 16 bytes apart. Distinct typed roots follow
/// the same C object/restrict rules as the vectorizer. Unknown roots (notably
/// loaded/call-returned pointers) fail closed instead of being mistaken for
/// pointers outside the current stack frame.
fn provably_disjoint(func: &IrFunction, src: Value, dst: Value, facts: &ModuleSymbolFacts) -> bool {
    if src == dst {
        return true;
    }
    let Some((src_root, src_offset)) = root_and_offset(func, src) else {
        return false;
    };
    let Some((dst_root, dst_offset)) = root_and_offset(func, dst) else {
        return false;
    };
    if same_root(&src_root, &dst_root) {
        return match (src_offset, dst_offset) {
            (Some(src_offset), Some(dst_offset)) => {
                dst_offset.checked_sub(src_offset).is_some_and(|distance| {
                    distance == 0 || distance.unsigned_abs() >= COPY_BYTES as u64
                })
            }
            _ => false,
        };
    }
    roots_proven_distinct(&src_root, &dst_root, facts)
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    run_with_symbol_facts(func, &ModuleSymbolFacts::default())
}

pub(crate) fn run_with_symbol_facts(func: &mut IrFunction, facts: &ModuleSymbolFacts) -> usize {
    let mut total = 0;
    // Each successful rewrite removes at least five instructions, so the loop
    // terminates; the bound is a belt-and-braces guard for a malformed IR.
    for _ in 0..4096 {
        let Some(c) = find_one(func, facts) else {
            break;
        };
        apply(func, &c);
        total += 1;
    }
    let mut collapsed = 0;
    // Round-trip forwarding can expose a carrier collapse and vice versa
    // (a forwarded vector feeds a staging slot whose reload was the only
    // thing keeping the carrier split alive), so iterate the pair to a
    // fixpoint; each round removes at least one instruction, and the bound
    // is a belt-and-braces guard against malformed IR.
    //
    // The purge runs INSIDE the loop: a collapse or forward removes the
    // only READ of a staging home, and the dead store left behind is often
    // the one non-return use that keeps the next collapse fail-closed.
    // Observed on `__m128i v = _mm_setzero_si128(); v = f(p); return v;`:
    // the forwarder hands the carrier reload to the home's reader, the
    // reload's remaining use is the (now unreadable) home store, and only
    // after that store is purged does the carrier collapse see a
    // return-only reload.  The generic DSE phases all ran before this
    // pass, so nobody else retires these stores — unpurged they survive
    // to codegen as two stores plus the frame slot (measured: 12
    // instructions instead of 5 on `return f()`).
    for _ in 0..8 {
        let round = forward_vector_roundtrips(func) + collapse_carrier_reloads(func);
        if round == 0 {
            break;
        }
        collapsed += round;
        purge_unread_local_stores(func);
    }
    total + collapsed
}

/// Delete every store to a stack object that is provably never read and
/// never escapes — and then the object itself (alloca plus derived GEPs),
/// unless it is a parameter home the prologue writes behind the IR's back.
///
/// The proof is a closed-world walk over the object's pointer web, fail
/// closed at every step: starting from each non-volatile `Alloca`, a derived
/// pointer may be used ONLY as a `GetElementPtr` base (extending the web) or
/// as the POINTER of a plain non-volatile store whose value operand is not
/// itself in the web.  Any other use — a load of any kind, an intrinsic, a
/// call argument, a store of the pointer AS a value, a terminator operand, a
/// volatile or non-default-address-space access, a GEP whose OFFSET is in
/// the web — keeps the object and every store to it.  Multi-definition ids
/// are refused outright, same as every other proof in this pass.
fn purge_unread_local_stores(func: &mut IrFunction) {
    let multi_def = super::gvn::find_multi_def_values(func);
    let mut allocas: Vec<Value> = Vec::new();
    // Every value id read anywhere (instruction operands and terminators):
    // the zero-fill intrinsics carry a VALUE destination alongside their
    // pointer write, and deleting one is only sound when that value is
    // provably unused.
    let mut used_values: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|v| {
                used_values.insert(v);
            });
            if let Instruction::Alloca {
                dest,
                volatile: false,
                semantic_volatile: false,
                ..
            } = inst
            {
                if !multi_def.contains(&dest.0) {
                    allocas.push(*dest);
                }
            }
        }
        block.terminator.for_each_used_value(|v| {
            used_values.insert(v);
        });
    }

    'alloca: for root in allocas {
        // Pointer web: the alloca plus every GEP chained off it.
        let mut web: FxHashSet<u32> = FxHashSet::default();
        web.insert(root.0);
        loop {
            let before = web.len();
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } = inst
                    {
                        if web.contains(&base.0)
                            && !matches!(offset, Operand::Value(v) if web.contains(&v.0))
                            && !multi_def.contains(&dest.0)
                        {
                            web.insert(dest.0);
                        }
                    }
                }
            }
            if web.len() == before {
                break;
            }
        }

        // Every use of every web pointer must be a GEP base (already walked)
        // or the pointer of a deletable plain store.
        for block in &func.blocks {
            for inst in &block.instructions {
                let mut touches = false;
                inst.for_each_used_value(|v| {
                    if web.contains(&v) {
                        touches = true;
                    }
                });
                if !touches {
                    continue;
                }
                let ok = match inst {
                    // The GEP's own result must have made it INTO the web
                    // (multi-def dests were refused above): otherwise the
                    // derived pointer is an untracked alias of the object.
                    Instruction::GetElementPtr {
                        dest, base, offset, ..
                    } => {
                        web.contains(&dest.0)
                            && web.contains(&base.0)
                            && !matches!(offset, Operand::Value(v) if web.contains(&v.0))
                    }
                    Instruction::Store {
                        ptr,
                        val,
                        seg_override: AddressSpace::Default,
                        volatile: false,
                        ..
                    } => {
                        web.contains(&ptr.0)
                            && !matches!(val, Operand::Value(v) if web.contains(&v.0))
                    }
                    // A whole-vector store INTO the object: a plain write,
                    // deletable like `Store` — the stored value must not be
                    // a web pointer and both pointer slots must stay inside
                    // the web (the intrinsic names the destination twice).
                    Instruction::Intrinsic {
                        dest: None,
                        op,
                        dest_ptr: Some(dp),
                        args,
                    } if is_vec16_store_op(*op) => {
                        web.contains(&dp.0)
                            && matches!(
                                args.as_slice(),
                                [val, Operand::Value(p2), _]
                                    if web.contains(&p2.0)
                                        && !matches!(val, Operand::Value(v) if web.contains(&v.0))
                            )
                    }
                    // A zero-fill INTO the object (`_mm_setzero_*` written
                    // straight to the variable's home): a plain write of
                    // constant bytes, deletable like `Store` — but only when
                    // its VALUE destination is provably unused, because the
                    // instruction also defines the zero vector as an SSA
                    // value and deleting a used definition is never sound.
                    Instruction::Intrinsic {
                        dest: Some(zdest),
                        op,
                        dest_ptr: Some(dp),
                        args,
                    } if is_zero_fill_op(*op) => {
                        args.is_empty() && web.contains(&dp.0) && !used_values.contains(&zdest.0)
                    }
                    // A whole-vector load FROM the object whose result is
                    // provably unused: a read in form only.  The collapse
                    // rounds orphan these (the forwarder rewrote the load's
                    // consumers), and left in place the dead read pins the
                    // object's zero-prefill — observed as a lone `pxor`
                    // plus a frame slot after everything else collapsed.
                    Instruction::Intrinsic {
                        dest: Some(ld),
                        op,
                        dest_ptr: None,
                        args,
                    } if is_vec16_load_op(*op) => {
                        matches!(
                            args.as_slice(),
                            [Operand::Value(p), Operand::Const(_)] if web.contains(&p.0)
                        ) && !used_values.contains(&ld.0)
                    }
                    _ => false,
                };
                if !ok {
                    continue 'alloca;
                }
            }
            let mut in_term = false;
            block.terminator.for_each_used_value(|v| {
                if web.contains(&v) {
                    in_term = true;
                }
            });
            if in_term {
                continue 'alloca;
            }
        }

        // Proven write-only and local: every store to the web is dead, and
        // once the stores are gone nothing reads a web pointer at all (the
        // proof above admitted ONLY GEP-base and store-pointer uses), so the
        // GEPs and the alloca itself are dead too.  Parameter homes are the
        // one exception: the prologue writes them through `param_alloca_values`
        // identity, so the slot must survive even when IR-visible stores die.
        // Retiring the object matters as much as retiring the stores — a
        // surviving 16-byte/16-align alloca costs 32 frame bytes (slot plus
        // alignment padding) that no later pass reclaims, because generic DCE
        // must preserve allocas in any function that contains a call
        // (measured on `_mm_add_epi32(f(p), w)`: 184- vs 152-byte frame).
        let drop_object = !func.param_alloca_values.iter().any(|pv| *pv == root);
        for block in &mut func.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                let drop = match &block.instructions[i] {
                    Instruction::Store { ptr, .. } => web.contains(&ptr.0),
                    Instruction::Intrinsic {
                        dest: None,
                        op,
                        dest_ptr: Some(dp),
                        ..
                    } => is_vec16_store_op(*op) && web.contains(&dp.0),
                    Instruction::Intrinsic {
                        dest: Some(_),
                        op,
                        dest_ptr: Some(dp),
                        ..
                    } => is_zero_fill_op(*op) && web.contains(&dp.0),
                    Instruction::Intrinsic {
                        dest: Some(ld),
                        op,
                        dest_ptr: None,
                        args,
                    } => {
                        is_vec16_load_op(*op)
                            && matches!(
                                args.as_slice(),
                                [Operand::Value(p), Operand::Const(_)] if web.contains(&p.0)
                            )
                            && !used_values.contains(&ld.0)
                    }
                    Instruction::Alloca { dest, .. } | Instruction::GetElementPtr { dest, .. } => {
                        drop_object && web.contains(&dest.0)
                    }
                    _ => false,
                };
                if drop {
                    block.remove_inst(i);
                } else {
                    i += 1;
                }
            }
        }
    }
}

/// Collapse the OTHER direction of the same carrier round trip: an `I128`
/// SSA value (typically an `[Sse, SseUp]` call result) split into two `I64`
/// halves, staged through a stack home, and immediately read back whole:
///
/// ```text
///   %lo = cast %x           (I128 -> I64)
///   store %lo  -> %a        (I64)
///   %sh = lshr %x, 64       (I128)
///   %hi = cast %sh          (I128 -> I64)
///   store %hi  -> %a + 8    (I64)
///   %v  = VecLoadI64x2 %a   ; reads exactly the 16 bytes just stored
/// ```
///
/// `%v` is bit-for-bit `%x` (lane 0 = bits 63:0, lane 1 = bits 127:64 —
/// little-endian lane order is the memory order the two stores wrote), so
/// every use of `%v` is rewritten to `%x` and the load is dropped.  The
/// stores, casts, shift and home then die by the ordinary dead-code rules
/// once nothing reads the home.  This is the `return f()` / `struct wrap
/// w = { f() };` shape for a 16-byte vector aggregate: without the rewrite
/// the call result takes a `movdqu` + two 64-bit reloads + two stores + a
/// `movdqu` round trip that costs ten instructions and two
/// store-to-load-forwarding stalls on the return path (RH-09iii).
///
/// Fail-closed discipline, same as `find_one`:
///
/// * every pattern piece sits in ONE block, the stores strictly before the
///   load;
/// * the addresses are proven with `offset_from` (same provenance rules as
///   the forward match): low store at the load address, high store exactly
///   8 bytes above;
/// * no memory-motion barrier between the first matched store and the
///   load, the two matched stores themselves excepted — any other store,
///   call, fence, atomic or volatile access declines;
/// * both stores are plain `I64`/default-address-space/non-volatile;
/// * `%x` and `%v` are single-definition values (`find_multi_def_values`),
///   so the substitution cannot change a position-dependent web.
/// The four whole-register 16-byte vector LOAD intrinsics.  They differ
/// only in lane interpretation; all read exactly 16 bytes at `ptr + off`
/// into one XMM-class SSA value, so for bit-identity reasoning they are
/// interchangeable.
fn is_vec16_load_op(op: crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as I;
    matches!(
        op,
        I::VecLoadI64x2 | I::VecLoadI32x4 | I::VecLoadF32x4 | I::VecLoadF64x2
    )
}

/// The four whole-register 16-byte vector STORE intrinsics (same lane
/// interpretations as [`is_vec16_load_op`]; all write exactly 16 bytes).
fn is_vec16_store_op(op: crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as I;
    matches!(
        op,
        I::VecStoreI64x2 | I::VecStoreI32x4 | I::VecStoreF32x4 | I::VecStoreF64x2
    )
}

/// The zero-fill intrinsics: a constant write of 16/32/64 zero bytes to
/// `dest_ptr` that ALSO defines the zero vector as an SSA value.  For the
/// write-only purge they are plain stores; the value destination is the
/// caller's problem (deletable only when provably unused).
fn is_zero_fill_op(op: crate::ir::intrinsics::IntrinsicOp) -> bool {
    use crate::ir::intrinsics::IntrinsicOp as I;
    matches!(
        op,
        I::Setzero128 | I::SetzeroPs128 | I::Setzero256 | I::Setzero512
    )
}

/// Replace every use of `v` with `x` (instruction operands and terminators,
/// all blocks) and remove `v`'s defining instruction.  `v` must be a
/// single-definition value; `x`'s definition dominates `v`'s by
/// construction in both callers (same block, strictly earlier).
fn substitute_and_drop(func: &mut IrFunction, v: Value, x: Value) {
    for block in &mut func.blocks {
        let drop_at = block
            .instructions
            .iter()
            .position(|inst| inst.dest() == Some(v));
        if let Some(at) = drop_at {
            block.remove_inst(at);
        }
        for inst in &mut block.instructions {
            inst.for_each_operand_mut(|op| {
                if matches!(op, Operand::Value(u) if *u == v) {
                    *op = Operand::Value(x);
                }
            });
        }
        block.terminator.for_each_operand_mut(|op| {
            if matches!(op, Operand::Value(u) if *u == v) {
                *op = Operand::Value(x);
            }
        });
    }
}

/// Forward a whole-vector store straight to a whole-vector reload of the
/// same 16 bytes:
///
/// ```text
///   VecStore<T>  %val -> %p + off      ; writes all 16 bytes
///   ...                                ; no memory-motion barrier
///   %v = VecLoad<U> %p + off           ; reads the same 16 bytes
/// ```
///
/// `%v` is bit-for-bit `%val` — both live in the XMM register file and the
/// lane types only label the bits — so every use of `%v` is rewritten to
/// `%val` and the load is dropped.  This is the post-vectorizer staging
/// shape (`_mm_add_epi32(f(p), w)` stores the sum into a private home and
/// reloads it for the return; `_mm_set1_ps(x)` is stored and reloaded as
/// I64x2 before a call), measured at two instructions and one
/// store-to-load-forwarding stall per occurrence.
///
/// Fail-closed discipline, same as the carrier collapse: single block,
/// backward scan from the load so the LATEST store wins, any intervening
/// memory-motion barrier declines (every other store IS a barrier, so a
/// clobbered slot can never forward), addresses proven with `offset_from`,
/// single-definition values only, and the argument shapes are matched
/// exactly (`[val, ptr, off]` / `[ptr, off]` with constant offsets) so any
/// future intrinsic signature change fails the match instead of silently
/// mis-pairing.
fn forward_vector_roundtrips(func: &mut IrFunction) -> usize {
    let mut total = 0;
    loop {
        let Some((v, val)) = find_vector_roundtrip(func) else {
            break;
        };
        substitute_and_drop(func, v, val);
        total += 1;
    }
    total
}

/// Locate one forwardable store/reload pair: `(reload dest, stored value)`.
fn find_vector_roundtrip(func: &IrFunction) -> Option<(Value, Value)> {
    let multi_def = super::gvn::find_multi_def_values(func);
    for block in &func.blocks {
        'load: for (li, inst) in block.instructions.iter().enumerate() {
            let Instruction::Intrinsic {
                dest: Some(v),
                op: load_op,
                dest_ptr: None,
                args,
            } = inst
            else {
                continue;
            };
            if !is_vec16_load_op(*load_op) || multi_def.contains(&v.0) {
                continue;
            }
            let [Operand::Value(q), off_arg] = args.as_slice() else {
                continue;
            };
            let load_off = match off_arg {
                Operand::Const(IrConst::I64(n)) => *n,
                Operand::Const(IrConst::I32(n)) => i64::from(*n),
                _ => continue,
            };

            // Walk BACKWARD: the first store hit is the latest one — the
            // one whose bytes the load observes.  Anything else that can
            // move memory declines before an older store can mis-match.
            for si in (0..li).rev() {
                let cand = &block.instructions[si];
                if let Instruction::Intrinsic {
                    dest: None,
                    op: store_op,
                    dest_ptr: Some(_),
                    args: sargs,
                } = cand
                {
                    if is_vec16_store_op(*store_op) {
                        let [Operand::Value(val), Operand::Value(p2), soff_arg] = sargs.as_slice()
                        else {
                            continue 'load;
                        };
                        let store_off = match soff_arg {
                            Operand::Const(IrConst::I64(n)) => *n,
                            Operand::Const(IrConst::I32(n)) => i64::from(*n),
                            _ => continue 'load,
                        };
                        let Some(delta) = offset_from(func, *q, *p2) else {
                            continue 'load; // may alias: fail closed
                        };
                        if delta + load_off != store_off || multi_def.contains(&val.0) || *val == *v
                        {
                            continue 'load;
                        }
                        // The forwarded value must come from a vector
                        // intrinsic (broadcast, arithmetic, load…): a true
                        // XMM-class SSA value every consumer can read.  An
                        // I128 call carrier stored via VecStore would trip
                        // the same backend hole the carrier collapse fails
                        // closed on (see collapse_carrier_reloads).
                        let vector_producer = matches!(
                            unique_def(func, *val),
                            Some(Instruction::Intrinsic { dest: Some(_), .. })
                        );
                        if !vector_producer {
                            continue 'load;
                        }
                        return Some((*v, *val));
                    }
                }
                if memory_motion_barrier(cand) {
                    continue 'load;
                }
            }
        }
    }
    None
}

fn collapse_carrier_reloads(func: &mut IrFunction) -> usize {
    let mut total = 0;
    loop {
        let Some((v, x)) = find_carrier_reload(func) else {
            break;
        };
        substitute_and_drop(func, v, x);
        total += 1;
    }
    total
}

/// Locate one collapsible split/reload: returns `(reload dest, carrier)`.
fn find_carrier_reload(func: &IrFunction) -> Option<(Value, Value)> {
    let multi_def = super::gvn::find_multi_def_values(func);
    for block in &func.blocks {
        'load: for (li, inst) in block.instructions.iter().enumerate() {
            let Instruction::Intrinsic {
                dest: Some(v),
                op: load_op,
                dest_ptr: None,
                args,
            } = inst
            else {
                continue;
            };
            // Any whole-register 16-byte load: the lane type only labels
            // the bits (observed live: the vectorizer reloads the carrier
            // staging as I32x4 when the consumer is a 32-bit lane op).
            if !is_vec16_load_op(*load_op) || multi_def.contains(&v.0) {
                continue;
            }
            let [Operand::Value(p), off_arg] = args.as_slice() else {
                continue;
            };
            let load_off = match off_arg {
                Operand::Const(IrConst::I64(n)) => *n,
                Operand::Const(IrConst::I32(n)) => i64::from(*n),
                _ => continue,
            };

            // The two carrier-half stores feeding the loaded bytes.
            let mut lo: Option<(usize, Value)> = None; // (store idx, carrier)
            let mut hi: Option<(usize, Value)> = None;
            for (si, st) in block.instructions.iter().enumerate().take(li) {
                let Instruction::Store {
                    val: Operand::Value(sv),
                    ptr,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } = st
                else {
                    continue;
                };
                let Some(Instruction::Cast {
                    src: Operand::Value(cast_src),
                    from_ty: IrType::I128,
                    to_ty: IrType::I64,
                    ..
                }) = unique_def(func, *sv)
                else {
                    continue;
                };
                let Some(delta) = offset_from(func, *ptr, *p) else {
                    continue;
                };
                if delta == load_off {
                    // Low half: the cast reads the carrier directly.
                    if !multi_def.contains(&cast_src.0) {
                        lo = Some((si, *cast_src));
                    }
                } else if delta == load_off + 8 {
                    // High half: the cast reads `lshr %x, 64`.
                    if let Some(Instruction::BinOp {
                        op: IrBinOp::LShr,
                        lhs: Operand::Value(shift_src),
                        rhs,
                        ty: IrType::I128,
                        ..
                    }) = unique_def(func, *cast_src)
                    {
                        if is_cst(rhs, 64) && !multi_def.contains(&shift_src.0) {
                            hi = Some((si, *shift_src));
                        }
                    }
                }
            }
            let (Some((lo_i, lo_x)), Some((hi_i, hi_x))) = (lo, hi) else {
                continue;
            };
            if lo_x != hi_x {
                continue;
            }
            // The carrier must be a call return that ALREADY lives in an
            // XMM register (`ret_is_vector128`: SysV classes [Sse, SseUp]).
            // That is the whole safety argument for the substitution — the
            // reload's users receive the bit-identical value in the same
            // register file.  An [Integer, Integer] I128 (e.g. `__int128`
            // punned to a vector through a union) reaches this pass as a
            // single un-split `Store I128` today and so never matches, but
            // that is an accident of lowering, not a guarantee; encode the
            // requirement instead of inheriting it.
            let carrier_in_xmm = matches!(
                unique_def(func, lo_x),
                Some(
                    Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. }
                ) if info.ret_is_vector128
            );
            if !carrier_in_xmm {
                continue;
            }
            // Every use of the reload must be a RETURN operand: the
            // backend's vector128 return path consumes an I128 carrier
            // directly (proven by `return f()` probes), but its vector
            // INTRINSIC operand path does not — substituting the carrier
            // into a `VecAddI32x4` argument materialized an unrelated
            // register (observed live: `movdqa %xmm15, %xmm0` reading an
            // uninitialized %xmm15).  Until the backend homes [Sse, SseUp]
            // call results for arbitrary vector consumers, any
            // non-terminator use fails closed.
            let mut non_return_use = false;
            for b in &func.blocks {
                for i2 in &b.instructions {
                    i2.for_each_used_value(|u| {
                        if u == v.0 {
                            non_return_use = true;
                        }
                    });
                }
                let mut term_uses = false;
                b.terminator.for_each_used_value(|u| {
                    if u == v.0 {
                        term_uses = true;
                    }
                });
                if term_uses && !matches!(&b.terminator, Terminator::Return(_)) {
                    non_return_use = true;
                }
            }
            if non_return_use {
                continue;
            }

            // Nothing between the first matched store and the load may move
            // memory, the matched stores themselves excepted.
            let first = lo_i.min(hi_i);
            for (mi, mid) in block.instructions.iter().enumerate().take(li).skip(first) {
                if mi == lo_i || mi == hi_i {
                    continue;
                }
                if memory_motion_barrier(mid) {
                    continue 'load;
                }
            }
            return Some((*v, lo_x));
        }
    }
    None
}

fn find_one(func: &IrFunction, facts: &ModuleSymbolFacts) -> Option<Candidate> {
    let multi_def = super::gvn::find_multi_def_values(func);
    // Use counts over the whole function, so a single-use proof also covers
    // uses in other blocks.
    let mut uses: FxHashMap<u32, usize> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|id| *uses.entry(id).or_default() += 1);
        }
        block
            .terminator
            .for_each_used_value(|id| *uses.entry(id).or_default() += 1);
    }
    let single_use = |v: u32| uses.get(&v).copied().unwrap_or(0) == 1;

    for (bi, block) in func.blocks.iter().enumerate() {
        for (oi, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest: agg,
                op: IrBinOp::Or,
                lhs,
                rhs,
                ty: IrType::I128,
            } = inst
            else {
                continue;
            };
            // The Or must be `shl(hi) | lo`, possibly with the operands swapped.
            let mut sides = [(lhs, rhs), (rhs, lhs)];
            let mut matched = None;
            for (a, b) in sides.iter_mut() {
                let (Some(hi_src), Some(lo_src)) = (shl_source(block, a), lo_source(block, b))
                else {
                    continue;
                };
                matched = Some((hi_src, lo_src));
                break;
            }
            let Some((hi_src, lo_src)) = matched else {
                continue;
            };

            // Exactly two uses of `agg`: the low truncation and the 64-shift.
            let mut trunc = None;
            let mut shifted = None;
            for (ii, other) in block.instructions.iter().enumerate() {
                match other {
                    Instruction::Cast {
                        dest,
                        src: Operand::Value(s),
                        from_ty: IrType::I128,
                        to_ty: IrType::I64,
                    } if s.0 == agg.0 => trunc = Some((ii, dest.0)),
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::LShr,
                        lhs: Operand::Value(s),
                        rhs,
                        ty: IrType::I128,
                    } if s.0 == agg.0 && is_cst(rhs, 64) => shifted = Some((ii, dest.0)),
                    _ => {}
                }
            }
            let (Some((trunc_i, trunc_dest)), Some((shift_i, shift_dest))) = (trunc, shifted)
            else {
                continue;
            };
            if uses.get(&agg.0).copied().unwrap_or(0) != 2 {
                continue;
            }

            // The two stores, one for each half.
            let mut store_lo = None;
            let mut store_hi = None;
            for (ii, other) in block.instructions.iter().enumerate() {
                let Instruction::Store {
                    val: Operand::Value(v),
                    ptr,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } = other
                else {
                    continue;
                };
                if v.0 == trunc_dest && ii > trunc_i {
                    store_lo = Some((ii, *ptr));
                }
            }
            // The high half is `cast (lshr agg, 64)`; find that cast then its store.
            let mut hi_cast = None;
            for (ii, other) in block.instructions.iter().enumerate() {
                if let Instruction::Cast {
                    dest,
                    src: Operand::Value(s),
                    from_ty: IrType::I128,
                    to_ty: IrType::I64,
                } = other
                {
                    if s.0 == shift_dest && ii > shift_i {
                        hi_cast = Some((ii, dest.0));
                    }
                }
            }
            let Some((hi_cast_i, hi_cast_dest)) = hi_cast else {
                continue;
            };
            for (ii, other) in block.instructions.iter().enumerate() {
                let Instruction::Store {
                    val: Operand::Value(v),
                    ptr,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                } = other
                else {
                    continue;
                };
                if v.0 == hi_cast_dest && ii > hi_cast_i {
                    store_hi = Some((ii, *ptr));
                }
            }
            let (Some((store_lo_i, q_lo)), Some((store_hi_i, q_hi))) = (store_lo, store_hi) else {
                continue;
            };

            // Every intermediate must be single-use, single-definition, and
            // the halves must be exactly 8 bytes apart on both sides.
            let mut hops = vec![
                lo_src.cast_dest,
                hi_src.cast_dest,
                hi_src.shift_dest,
                agg.0,
                trunc_dest,
                shift_dest,
                hi_cast_dest,
            ];
            hops.extend(lo_src.load_dest);
            hops.extend(hi_src.load_dest);
            if hops.iter().any(|id| multi_def.contains(id)) {
                continue;
            }
            if !single_use(hi_src.shift_dest)
                || !single_use(trunc_dest)
                || !single_use(hi_cast_dest)
                || !single_use(lo_src.cast_dest)
                || !single_use(hi_src.cast_dest)
                || !single_use(shift_dest)
                || lo_src.load_dest.is_some_and(|id| !single_use(id))
                || hi_src.load_dest.is_some_and(|id| !single_use(id))
            {
                continue;
            }

            // The two halves must be exactly 8 bytes apart on both sides.
            if offset_from(func, hi_src.ptr, lo_src.ptr) != Some(HALF)
                || offset_from(func, q_hi, q_lo) != Some(HALF)
            {
                continue;
            }
            if !provably_disjoint(func, lo_src.ptr, q_lo, facts) {
                continue;
            }
            let Some((p_root, _)) = root_and_offset(func, lo_src.ptr) else {
                continue;
            };
            let Some((q_root, _)) = root_and_offset(func, q_lo) else {
                continue;
            };

            // The read of `p` moves later (to the store site), so an
            // intervening write, ordering operation, or volatile access is
            // only harmless when we prove it cannot affect the source or
            // destination. Ordinary stores to a distinct stack home are the
            // sole refined case; every other effect fails closed.
            let first = lo_src.load_index.min(hi_src.load_index);
            let last = store_lo_i.max(store_hi_i);
            let replaced = [store_lo_i, store_hi_i];
            let mut blocked = false;
            for (ii, inst) in block.instructions.iter().enumerate() {
                if ii < first || ii > last || replaced.contains(&ii) || !memory_motion_barrier(inst)
                {
                    continue;
                }
                if let Instruction::Store {
                    ptr,
                    volatile: false,
                    seg_override: AddressSpace::Default,
                    ..
                } = inst
                {
                    let Some(write_root) = store_root(func, *ptr) else {
                        blocked = true;
                        break;
                    };
                    let aliases_p =
                        matches!(p_root, ObjectRoot::Alloca(root) if root == write_root);
                    let aliases_q =
                        matches!(q_root, ObjectRoot::Alloca(root) if root == write_root);
                    if aliases_p || aliases_q {
                        blocked = true;
                        break;
                    }
                } else {
                    blocked = true;
                    break;
                }
            }
            if blocked {
                continue;
            }

            let mut drop = vec![
                lo_src.load_index,
                hi_src.load_index,
                lo_src.cast_index,
                hi_src.cast_index,
                hi_src.shift_index,
                oi,
                trunc_i,
                shift_i,
                hi_cast_i,
                store_lo_i,
                store_hi_i,
            ];
            drop.extend(lo_src.extra);
            drop.extend(hi_src.extra);
            drop.sort_unstable();
            drop.dedup();
            return Some(Candidate {
                block: bi,
                drop,
                insert_at: last,
                dest_ptr: q_lo,
                src_ptr: lo_src.ptr,
            });
        }
    }
    None
}

/// The load side of one half: the `cast` of a load into `I128`.
#[derive(Clone)]
struct HalfLoad {
    ptr: Value,
    load_dest: Option<u32>,
    load_index: usize,
    cast_index: usize,
    cast_dest: u32,
    shift_index: usize,
    shift_dest: u32,
    extra: Vec<usize>,
}

/// `shl (cast %hi -> i128), 64`
fn shl_source(block: &crate::ir::reexports::BasicBlock, op: &Operand) -> Option<HalfLoad> {
    let Operand::Value(sh) = op else { return None };
    for (si, inst) in block.instructions.iter().enumerate() {
        let Instruction::BinOp {
            dest,
            op: IrBinOp::Shl,
            lhs,
            rhs,
            ty: IrType::I128,
        } = inst
        else {
            continue;
        };
        if dest.0 != sh.0 || !is_cst(rhs, 64) {
            continue;
        }
        let (ci, cd, ptr, ld, li) = cast_of_load(block, lhs)?;
        return Some(HalfLoad {
            ptr,
            load_dest: ld,
            load_index: li,
            cast_index: ci,
            cast_dest: cd,
            shift_index: si,
            shift_dest: dest.0,
            extra: Vec::new(),
        });
    }
    None
}

/// `cast (load %lo) -> i128`
fn lo_source(block: &crate::ir::reexports::BasicBlock, op: &Operand) -> Option<HalfLoad> {
    let (ci, cd, ptr, ld, li) = cast_of_load(block, op)?;
    Some(HalfLoad {
        ptr,
        load_dest: ld,
        load_index: li,
        cast_index: ci,
        cast_dest: cd,
        shift_index: usize::MAX,
        shift_dest: 0,
        extra: Vec::new(),
    })
}

/// Resolve `cast (load %p) -> i128` to (cast index, cast dest, ptr, load dest,
/// load index), plus any `gep` that only feeds the load.
fn cast_of_load(
    block: &crate::ir::reexports::BasicBlock,
    op: &Operand,
) -> Option<(usize, u32, Value, Option<u32>, usize)> {
    let Operand::Value(cv) = op else { return None };
    for (ci, inst) in block.instructions.iter().enumerate() {
        let Instruction::Cast {
            dest,
            src: Operand::Value(s),
            from_ty,
            to_ty: IrType::I128,
        } = inst
        else {
            continue;
        };
        // This is a bit-preserving pack of an I64 load into the low/high
        // half of an I128 carrier. A signed I64 -> I128 cast sign-extends;
        // accepting it (especially on the low half) would let the `or` smear
        // sign bits into the other half, so only the explicit zero-extension
        // form is equivalent to a raw 16-byte copy.
        if dest.0 != cv.0 || *from_ty != IrType::U64 {
            continue;
        }
        for (li, other) in block.instructions.iter().enumerate() {
            let Instruction::Load {
                dest: ld,
                ptr,
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
                volatile: false,
            } = other
            else {
                continue;
            };
            if ld.0 == s.0 {
                return Some((ci, cv.0, *ptr, Some(ld.0), li));
            }
        }
    }
    None
}

fn apply(func: &mut IrFunction, c: &Candidate) {
    // The replacement is the *intrinsic* 128-bit copy pair, not a `Memcpy`.
    // `Loaddqu`/`Storedqu` are what `vector_temp_promotion::fuse_vector_loads`
    // recognises as a forwardable vector producer and consumer, so the copy
    // this pattern implemented then disappears into the consuming SIMD
    // operation's memory operand instead of being emitted at all:
    //
    //   Loaddqu  %h <- %p          ==>  AddPd128 reads %p directly
    //   AddPd128 %r <- %h1, %h2         (no %h1/%h2 slot traffic)
    //   Storedqu %q <- %r          ==>  the result is stored to %q directly
    //
    // A `Memcpy` here would be correct but opaque to that forwarding.
    let block = &mut func.blocks[c.block];
    let (old_spans, old_ids) = block.take_parallel_debug();
    let tracked = !old_spans.is_empty();
    let mut rewritten = Vec::with_capacity(block.instructions.len());
    let mut spans = Vec::with_capacity(if tracked { rewritten.capacity() } else { 0 });
    let mut ids = Vec::with_capacity(if tracked { rewritten.capacity() } else { 0 });
    for (ii, inst) in block.instructions.iter().enumerate() {
        if ii == c.insert_at {
            if tracked {
                // The vector load stands in for the pair it replaces.
                spans.push(old_spans[ii]);
                ids.push(
                    old_ids
                        .get(ii)
                        .copied()
                        .unwrap_or(crate::ir::instruction::NO_INLINE_INSTANCE),
                );
            }
            rewritten.push(Instruction::Intrinsic {
                // `Loaddqu` writes the vector to `dest_ptr`; it has no SSA
                // result (the backend does not define `dest`).
                dest: None,
                op: crate::ir::intrinsics::IntrinsicOp::Loaddqu,
                dest_ptr: Some(c.dest_ptr),
                args: vec![Operand::Value(c.src_ptr)],
            });
        }
        if c.drop.binary_search(&ii).is_ok() {
            continue;
        }
        rewritten.push(inst.clone());
        if tracked {
            spans.push(old_spans[ii]);
            ids.push(
                old_ids
                    .get(ii)
                    .copied()
                    .unwrap_or(crate::ir::instruction::NO_INLINE_INSTANCE),
            );
        }
    }
    if old_ids.is_empty() {
        ids.clear();
    }
    block.replace_insts_inline(rewritten, spans, ids);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType as T;
    use crate::ir::reexports::{
        BasicBlock, BlockId, Instruction, IrConst, Operand, Terminator, Value,
    };

    fn func_with(insts: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("t".to_string(), T::Void, Vec::new(), false);
        f.params = vec![crate::ir::reexports::IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            fp_agg_class: None,
        }];
        f.next_value_id = 256;
        f.blocks = vec![BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }];
        f
    }

    fn param_ref(dest: u32, param_idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(dest),
            param_idx,
            ty: IrType::Ptr,
        }
    }

    fn global_addr(dest: u32, name: &str) -> Instruction {
        Instruction::GlobalAddr {
            dest: Value(dest),
            name: name.to_string(),
        }
    }

    fn alloca(dest: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn cast(dest: u32, src: u32, from: IrType, to: IrType) -> Instruction {
        Instruction::Cast {
            dest: Value(dest),
            src: Operand::Value(Value(src)),
            from_ty: from,
            to_ty: to,
        }
    }

    fn store(val: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn store_const(ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Const(IrConst::I64(0)),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn store_ptr(val: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn load_ptr(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn bin(op: IrBinOp, dest: u32, lhs: u32, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(lhs)),
            rhs,
            ty,
        }
    }

    fn gep(dest: u32, base: u32, off: i64) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(off)),
            ty: IrType::I8,
        }
    }

    /// The exact shape `_mm_loadu_pd` + a 16-byte store lowers to today.
    /// `src` is a pointer parameter and `dst` is a local 16-byte home.
    fn roundtrip(src: u32, dst: u32) -> Vec<Instruction> {
        roundtrip_or_order(src, dst, false)
    }

    fn global_roundtrip(src: &str, dst: &str) -> Vec<Instruction> {
        let mut insts = vec![global_addr(2, src), global_addr(3, dst)];
        insts.extend(roundtrip_body(2, 3, false));
        insts
    }

    fn roundtrip_or_order(src: u32, dst: u32, swap_or: bool) -> Vec<Instruction> {
        let mut insts = vec![param_ref(src, 0), alloca(dst, 16)];
        insts.extend(roundtrip_body(src, dst, swap_or));
        insts
    }

    /// The pack/unpack sequence without pointer definitions. Tests use it to
    /// build aliasing roots other than an incoming parameter.
    fn roundtrip_body(src: u32, dst: u32, swap_or: bool) -> Vec<Instruction> {
        vec![
            load(10, src),
            gep(11, src, 8),
            load(12, 11),
            cast(13, 12, IrType::U64, IrType::I128),
            cast(14, 10, IrType::U64, IrType::I128),
            bin(
                IrBinOp::Shl,
                15,
                13,
                Operand::Const(IrConst::I64(64)),
                IrType::I128,
            ),
            if swap_or {
                bin(IrBinOp::Or, 16, 14, Operand::Value(Value(15)), IrType::I128)
            } else {
                bin(IrBinOp::Or, 16, 15, Operand::Value(Value(14)), IrType::I128)
            },
            cast(17, 16, IrType::I128, IrType::I64),
            store(17, dst),
            bin(
                IrBinOp::LShr,
                18,
                16,
                Operand::Const(IrConst::I64(64)),
                IrType::I128,
            ),
            cast(19, 18, IrType::I128, IrType::I64),
            gep(20, dst, 8),
            store(19, 20),
        ]
    }

    #[test]
    fn a_sixteen_byte_round_trip_becomes_one_128_bit_load() {
        let mut f = func_with(roundtrip(2, 3));
        let next_value_id = f.next_value_id;
        assert_eq!(run(&mut f), 1, "the canonical shape must be recognised");
        assert_eq!(
            f.next_value_id, next_value_id,
            "the memory copy has no SSA result"
        );
        let insts = &f.blocks[0].instructions;
        // The rewrite is the intrinsic 128-bit copy pair (`Loaddqu`), which the
        // later vector-load forwarding turns into the consumer's memory
        // operand instead of emitting the copy at all.
        let loads: Vec<&Instruction> = insts
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Intrinsic {
                        dest: None,
                        op: crate::ir::intrinsics::IntrinsicOp::Loaddqu,
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(
            loads.len(),
            1,
            "expected one 16-byte Loaddqu, got {insts:?}"
        );
        assert!(
            matches!(
                loads[0],
                Instruction::Intrinsic {
                    dest_ptr: Some(Value(3)),
                    args,
                    ..
                } if matches!(args.as_slice(), [Operand::Value(Value(2))])
            ),
            "the copy must read the source pointer into the destination home"
        );
        // None of the integer-carrier work may survive.
        assert!(
            !insts.iter().any(|i| matches!(
                i,
                Instruction::Store { .. }
                    | Instruction::Load { .. }
                    | Instruction::BinOp {
                        ty: IrType::I128,
                        ..
                    }
            )),
            "the I128 pack/unpack must be gone: {insts:?}"
        );
    }

    #[test]
    fn a_swapped_or_operand_order_is_recognised() {
        let insts = roundtrip_or_order(2, 3, true);
        let mut f = func_with(insts);
        assert_eq!(
            run(&mut f),
            1,
            "or(lo, shl) must collapse as well as or(shl, lo)"
        );
        assert!(
            !f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::BinOp {
                    ty: IrType::I128,
                    ..
                }
            )),
            "swapped operand order must collapse too"
        );
    }

    fn insert_after_high_load(insts: &mut Vec<Instruction>, effect: Instruction) {
        let high_load = insts
            .iter()
            .position(|inst| {
                matches!(
                    inst,
                    Instruction::Load {
                        dest: Value(12),
                        ..
                    }
                )
            })
            .expect("high-half source load");
        insts.insert(high_load + 1, effect);
    }

    #[test]
    fn an_unknown_store_between_the_loads_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        insert_after_high_load(&mut insts, store_const(99));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "an unproven intervening store must refuse");
    }

    #[test]
    fn a_store_to_a_proven_distinct_stack_home_is_allowed() {
        let mut insts = roundtrip(2, 3);
        insert_after_high_load(&mut insts, alloca(30, 8));
        let at = insts
            .iter()
            .position(|inst| {
                matches!(
                    inst,
                    Instruction::Alloca {
                        dest: Value(30),
                        ..
                    }
                )
            })
            .unwrap()
            + 1;
        insts.insert(at, store_const(30));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 1, "a disjoint local store should not block");
    }

    #[test]
    fn strong_private_globals_can_prove_a_nonoverlapping_copy() {
        let facts = ModuleSymbolFacts {
            aliased_names: FxHashSet::default(),
            identity_witnesses: ["source", "destination"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ..ModuleSymbolFacts::default()
        };
        let mut f = func_with(global_roundtrip("source", "destination"));
        assert_eq!(
            run_with_symbol_facts(&mut f, &facts),
            1,
            "unredirected strong private globals retain the copy optimization"
        );
    }

    #[test]
    fn a_global_alias_is_not_mistaken_for_a_disjoint_object() {
        let facts = ModuleSymbolFacts {
            aliased_names: ["source", "alias_source"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            // Deliberately include both names: the alias marker, not just the
            // linkage class, must defeat the disjointness proof.
            identity_witnesses: ["source", "alias_source", "destination"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ..ModuleSymbolFacts::default()
        };
        let mut f = func_with(global_roundtrip("source", "alias_source"));
        assert_eq!(
            run_with_symbol_facts(&mut f, &facts),
            0,
            "a GNU alias can make distinct GlobalAddr names overlap"
        );
    }

    #[test]
    fn a_shared_intermediate_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        // A second reader of the packed aggregate.
        insts.push(cast(21, 16, IrType::I128, IrType::I64));
        insts.push(store(21, 98));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn overlapping_source_and_destination_keep_the_round_trip() {
        // src = base, dst = base + 8. The first destination half overwrites
        // the source's second half, so moving both source loads to the store
        // site would change the value being copied.
        let mut insts = vec![alloca(2, 32), gep(3, 2, 8)];
        insts.extend(roundtrip_body(2, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "a partial overlap must refuse");
    }

    #[test]
    fn a_copy_chain_cannot_hide_a_partial_stack_alias() {
        let mut insts = vec![alloca(2, 32), gep(4, 2, 0)];
        insts.push(Instruction::Copy {
            dest: Value(5),
            src: Operand::Value(Value(4)),
        });
        insts.push(gep(3, 2, 8));
        insts.extend(roundtrip_body(5, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "Copy must not hide the shared alloca root");
    }

    #[test]
    fn a_pointer_cast_chain_cannot_hide_a_partial_stack_alias() {
        let mut insts = vec![alloca(2, 32), gep(4, 2, 0)];
        insts.push(Instruction::Cast {
            dest: Value(5),
            src: Operand::Value(Value(4)),
            from_ty: IrType::Ptr,
            to_ty: IrType::Ptr,
        });
        insts.push(gep(3, 2, 8));
        insts.extend(roundtrip_body(5, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "Ptr-to-Ptr casts must preserve provenance");
    }

    #[test]
    fn a_loaded_pointer_is_not_assumed_outside_the_current_frame() {
        // `src` is loaded from a local pointer slot and points at the same
        // stack object as `dst` (with an 8-byte offset). The old numeric-root
        // test classified this opaque load result as "not a stack home" and
        // incorrectly treated it as disjoint from the destination alloca.
        let mut insts = vec![
            alloca(2, 32),
            gep(3, 2, 8),
            alloca(4, 8),
            store_ptr(2, 4),
            load_ptr(5, 4),
        ];
        insts.extend(roundtrip_body(5, 3, false));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "loaded pointers must fail closed");
    }

    #[test]
    fn extra_uses_of_either_source_load_prevent_deletion() {
        for load_value in [10, 12] {
            let mut insts = roundtrip(2, 3);
            insts.push(cast(30, load_value, IrType::U64, IrType::I128));
            let mut f = func_with(insts);
            assert_eq!(
                run(&mut f),
                0,
                "source load %{} has a second reader",
                load_value
            );
        }
    }

    #[test]
    fn an_extra_use_of_the_right_shift_result_prevents_deletion() {
        let mut insts = roundtrip(2, 3);
        let shift = insts
            .iter()
            .position(|inst| {
                matches!(
                    inst,
                    Instruction::BinOp {
                        dest: Value(18),
                        ..
                    }
                )
            })
            .unwrap();
        // Keep the candidate's original high cast after this extra use so it
        // remains the cast selected as the store-producing half.
        insts.insert(shift + 1, cast(30, 18, IrType::I128, IrType::I64));
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0, "the shift result is no longer single-use");
    }

    #[test]
    fn non_store_memory_and_ordering_effects_block_motion() {
        let effects = vec![
            Instruction::AtomicLoad {
                dest: Value(30),
                ptr: Operand::Value(Value(2)),
                ty: IrType::I64,
                ordering: crate::ir::ops::AtomicOrdering::SeqCst,
            },
            Instruction::Load {
                dest: Value(30),
                ptr: Value(2),
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
                volatile: true,
            },
            Instruction::InlineAsm {
                template: "".into(),
                outputs: Vec::new(),
                inputs: Vec::new(),
                clobbers: vec!["memory".into()],
                operand_types: Vec::new(),
                goto_labels: Vec::new(),
                input_symbols: Vec::new(),
                seg_overrides: Vec::new(),
            },
            Instruction::Intrinsic {
                dest: None,
                op: crate::ir::intrinsics::IntrinsicOp::Storedqu,
                dest_ptr: Some(Value(2)),
                args: vec![Operand::Value(Value(31))],
            },
            Instruction::PgoCounterInc {
                name: "counter".into(),
                offset: 0,
                atomic: false,
            },
            Instruction::Intrinsic {
                dest: None,
                op: crate::ir::intrinsics::IntrinsicOp::Mfence,
                dest_ptr: None,
                args: Vec::new(),
            },
        ];
        for effect in effects {
            let label = format!("{effect:?}");
            let mut insts = roundtrip(2, 3);
            insts.insert(0, alloca(31, 16));
            insert_after_high_load(&mut insts, effect);
            let mut f = func_with(insts);
            assert_eq!(run(&mut f), 0, "must not move a read across {label}");
        }
    }

    #[test]
    fn a_non_adjacent_pair_keeps_the_round_trip() {
        let mut insts = roundtrip(2, 3);
        let high_ptr = insts
            .iter_mut()
            .find(|inst| {
                matches!(
                    inst,
                    Instruction::GetElementPtr {
                        dest: Value(11),
                        ..
                    }
                )
            })
            .unwrap();
        *high_ptr = gep(11, 2, 16); // high half 16 bytes away, not 8
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn a_signed_low_half_is_not_mistaken_for_a_bitwise_copy() {
        let mut insts = roundtrip(2, 3);
        let low_cast = insts
            .iter_mut()
            .find(|inst| {
                matches!(
                    inst,
                    Instruction::Cast {
                        dest: Value(14),
                        ..
                    }
                )
            })
            .unwrap();
        *low_cast = cast(14, 10, IrType::I64, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(
            run(&mut f),
            0,
            "a signed widening cast can smear the low half's sign into the high half"
        );
    }

    #[test]
    fn a_32_bit_source_is_not_matched() {
        let mut insts = roundtrip(2, 3);
        let high_cast = insts
            .iter_mut()
            .find(|inst| {
                matches!(
                    inst,
                    Instruction::Cast {
                        dest: Value(13),
                        ..
                    }
                )
            })
            .unwrap();
        *high_cast = cast(13, 12, IrType::U32, IrType::I128);
        let mut f = func_with(insts);
        assert_eq!(run(&mut f), 0);
    }

    // ── Carrier reload collapse ─────────────────────────────────────────────
    //
    // The post-call shape of `return f()` for a 16-byte vector aggregate:
    // the I128 carrier is split into two 64-bit stores and immediately
    // reassembled by a `VecLoadI64x2` of the same staging slot.  The
    // collapse forwards the carrier to the load's users and then retires
    // the now-unreadable staging stores.

    fn vecload(dest: u32, ptr: u32, off: i64) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op: crate::ir::intrinsics::IntrinsicOp::VecLoadI64x2,
            dest_ptr: None,
            args: vec![
                Operand::Value(Value(ptr)),
                Operand::Const(IrConst::I64(off)),
            ],
        }
    }

    /// A call returning a 16-byte vector through the I128 carrier:
    /// SysV classes [Sse, SseUp], i.e. the value arrives in %xmm0.
    fn vec128_call(dest: u32, arg: u32) -> Instruction {
        Instruction::Call {
            func: "mk4".to_string(),
            info: crate::ir::reexports::CallInfo {
                dest: Some(Value(dest)),
                args: vec![Operand::Value(Value(arg))],
                arg_types: vec![IrType::Ptr],
                num_fixed_args: 1,
                return_type: IrType::I128,
                ret_is_vector128: true,
                ..Default::default()
            },
        }
    }

    /// carrier %5 (I128 vector128 call return), staging alloca %6,
    /// reload %11; `Return %11`.
    fn carrier_split(insts_before_load: &mut Vec<Instruction>) -> IrFunction {
        let mut insts = vec![
            param_ref(2, 0),
            vec128_call(5, 2),
            alloca(6, 16),
            cast(7, 5, IrType::I128, IrType::I64),
            store(7, 6),
            bin(
                IrBinOp::LShr,
                8,
                5,
                Operand::Const(IrConst::I64(64)),
                IrType::I128,
            ),
            cast(9, 8, IrType::I128, IrType::I64),
            gep(10, 6, 8),
            store(9, 10),
        ];
        insts.append(insts_before_load);
        insts.push(vecload(11, 6, 0));
        let mut f = func_with(insts);
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(11))));
        f
    }

    #[test]
    fn a_carrier_split_reload_collapses_to_the_carrier() {
        let mut f = carrier_split(&mut Vec::new());
        assert_eq!(run(&mut f), 1, "the collapse must be counted");
        let insts = &f.blocks[0].instructions;
        assert!(
            !insts.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::VecLoadI64x2,
                    ..
                }
            )),
            "the reload must be gone"
        );
        assert!(
            matches!(
                f.blocks[0].terminator,
                Terminator::Return(Some(Operand::Value(Value(5))))
            ),
            "the return must read the carrier directly: {:?}",
            f.blocks[0].terminator
        );
        assert!(
            !insts.iter().any(|i| matches!(i, Instruction::Store { .. })),
            "the staging stores are unreadable and must be purged: {insts:?}"
        );
    }

    #[test]
    fn a_zero_prefill_of_the_staging_home_is_purged_with_the_split() {
        // `__m128i v = _mm_setzero_si128(); v = f(p); return v;` leaves an
        // extra constant store into the same home before the split.
        let mut f = carrier_split(&mut Vec::new());
        f.blocks[0].instructions.insert(3, store_const(6));
        assert_eq!(run(&mut f), 1);
        assert!(
            !f.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Store { .. })),
            "the zero prefill writes a slot nobody reads any more"
        );
    }

    #[test]
    fn a_memory_barrier_between_split_and_reload_declines() {
        // A store through an unrelated pointer may alias the staging home's
        // observers; the window scan must refuse to forward across it.
        let mut f = carrier_split(&mut vec![store_const(2)]);
        assert_eq!(run(&mut f), 0);
        assert!(f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Intrinsic {
                op: crate::ir::intrinsics::IntrinsicOp::VecLoadI64x2,
                ..
            }
        )));
    }

    #[test]
    fn halves_of_two_different_carriers_decline() {
        let mut f = carrier_split(&mut Vec::new());
        // Retarget the high half's shift onto a second carrier %25.
        f.blocks[0].instructions.insert(2, vec128_call(25, 2));
        for inst in &mut f.blocks[0].instructions {
            if let Instruction::BinOp {
                dest: Value(8),
                lhs,
                ..
            } = inst
            {
                *lhs = Operand::Value(Value(25));
            }
        }
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn a_carrier_that_is_not_a_vector128_call_return_declines() {
        // An [Integer, Integer] I128 — `__int128` punned to a vector — must
        // not be forwarded into vector users: the value lives in a GP pair,
        // not an XMM, and the substitution's safety argument does not hold.
        let mut f = carrier_split(&mut Vec::new());
        for inst in &mut f.blocks[0].instructions {
            if let Instruction::Call { info, .. } = inst {
                info.ret_is_vector128 = false;
            }
        }
        assert_eq!(run(&mut f), 0, "GP-pair carriers must fail closed");

        // Same for a carrier that is not a call return at all.
        let mut g = carrier_split(&mut Vec::new());
        g.blocks[0].instructions[1] = cast(5, 2, IrType::U64, IrType::I128);
        assert_eq!(run(&mut g), 0, "non-call carriers must fail closed");
    }

    #[test]
    fn a_reload_at_the_wrong_offset_declines() {
        let mut f = carrier_split(&mut Vec::new());
        for inst in &mut f.blocks[0].instructions {
            if let Instruction::Intrinsic { args, .. } = inst {
                args[1] = Operand::Const(IrConst::I64(8));
            }
        }
        assert_eq!(run(&mut f), 0);
    }

    // ── Whole-vector round-trip forwarding ──────────────────────────────
    //
    // The post-vectorizer staging shape: a vector intrinsic result is
    // stored whole into a private slot and immediately reloaded whole
    // (possibly under a different lane label).  The forwarder hands the
    // producer's register value straight to the reload's users; the purge
    // then retires the store, the slot and the slot's alloca.

    fn vecstore_f32(val: u32, ptr: u32, off: i64) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op: crate::ir::intrinsics::IntrinsicOp::VecStoreF32x4,
            dest_ptr: Some(Value(ptr)),
            args: vec![
                Operand::Value(Value(val)),
                Operand::Value(Value(ptr)),
                Operand::Const(IrConst::I64(off)),
            ],
        }
    }

    fn broadcast_f32(dest: u32, src: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op: crate::ir::intrinsics::IntrinsicOp::VecBroadcastF32x4,
            dest_ptr: None,
            args: vec![Operand::Value(Value(src))],
        }
    }

    /// broadcast %5, staging alloca %6, store, reload %11; `Return %11`.
    fn vector_roundtrip(insts_between: &mut Vec<Instruction>) -> IrFunction {
        let mut insts = vec![
            param_ref(2, 0),
            alloca(6, 16),
            broadcast_f32(5, 2),
            vecstore_f32(5, 6, 0),
        ];
        insts.append(insts_between);
        insts.push(vecload(11, 6, 0));
        let mut f = func_with(insts);
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(11))));
        f
    }

    #[test]
    fn a_vector_roundtrip_forwards_the_stored_value() {
        let mut f = vector_roundtrip(&mut Vec::new());
        assert_eq!(run(&mut f), 1, "the forward must be counted");
        assert!(
            matches!(
                f.blocks[0].terminator,
                Terminator::Return(Some(Operand::Value(Value(5))))
            ),
            "the return must read the producer directly: {:?}",
            f.blocks[0].terminator
        );
        let insts = &f.blocks[0].instructions;
        assert!(
            !insts.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::VecLoadI64x2
                        | crate::ir::intrinsics::IntrinsicOp::VecStoreF32x4,
                    ..
                }
            )),
            "the store and the reload are both frame traffic: {insts:?}"
        );
        assert!(
            !insts
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { .. })),
            "the staging object is dead and must not cost frame bytes: {insts:?}"
        );
    }

    #[test]
    fn the_latest_store_wins_a_forwarded_roundtrip() {
        // Two whole-vector stores to the same slot: the reload observes the
        // SECOND one's bytes, so the backward scan must forward %15, not %5.
        let mut f = vector_roundtrip(&mut vec![broadcast_f32(15, 2), vecstore_f32(15, 6, 0)]);
        assert_eq!(run(&mut f), 1);
        assert!(
            matches!(
                f.blocks[0].terminator,
                Terminator::Return(Some(Operand::Value(Value(15))))
            ),
            "the reload observes the latest store: {:?}",
            f.blocks[0].terminator
        );
    }

    #[test]
    fn a_roundtrip_of_a_non_vector_producer_declines() {
        // The stored value is an I128 call carrier, not a vector intrinsic
        // result.  Forwarding it into the reload's users would hand an
        // [Sse, SseUp] carrier to consumers the backend only serves through
        // memory (observed live as a read of an uninitialized register:
        // `movdqa %xmm15, %xmm0`), so the producer gate must fail closed.
        let mut f = vector_roundtrip(&mut Vec::new());
        f.blocks[0].instructions[2] = vec128_call(5, 2);
        assert_eq!(run(&mut f), 0, "a call carrier is not a vector producer");
        assert!(f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Intrinsic {
                op: crate::ir::intrinsics::IntrinsicOp::VecLoadI64x2,
                ..
            }
        )));
    }

    #[test]
    fn a_barrier_between_vector_store_and_reload_declines() {
        // A store through an unrelated pointer may alias the slot.
        let mut f = vector_roundtrip(&mut vec![store_const(2)]);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn a_roundtrip_at_a_different_offset_declines() {
        let mut f = vector_roundtrip(&mut Vec::new());
        for inst in &mut f.blocks[0].instructions {
            if let Instruction::Intrinsic {
                dest: Some(Value(11)),
                args,
                ..
            } = inst
            {
                args[1] = Operand::Const(IrConst::I64(8));
            }
        }
        assert_eq!(run(&mut f), 0, "offset 8 reads bytes the store never wrote");
    }

    #[test]
    fn a_param_home_survives_the_dead_object_purge() {
        // Same proven-dead web, but the alloca is a parameter home: the
        // prologue writes it behind the IR's back, so only the IR-visible
        // store may die — the slot itself must stay.
        let mut f = vector_roundtrip(&mut Vec::new());
        f.param_alloca_values = vec![Value(6)];
        assert_eq!(run(&mut f), 1, "the forward itself still fires");
        assert!(
            f.blocks[0]
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { dest: Value(6), .. })),
            "a parameter home must survive: {:?}",
            f.blocks[0].instructions
        );
    }

    fn setzero(dest: u32, ptr: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op: crate::ir::intrinsics::IntrinsicOp::Setzero128,
            dest_ptr: Some(Value(ptr)),
            args: vec![],
        }
    }

    fn vecstore_i64(val: u32, ptr: u32, off: i64) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op: crate::ir::intrinsics::IntrinsicOp::VecStoreI64x2,
            dest_ptr: Some(Value(ptr)),
            args: vec![
                Operand::Value(Value(val)),
                Operand::Value(Value(ptr)),
                Operand::Const(IrConst::I64(off)),
            ],
        }
    }

    #[test]
    fn a_zero_initialized_home_collapses_through_the_purge_interleave() {
        // `__m128i v = _mm_setzero_si128(); v = f(p); return v;` — the
        // chain only collapses when the purge runs INSIDE the fixpoint:
        // the forwarder hands the carrier reload to the home's reader, the
        // reload's remaining use is the now-unreadable home store, and the
        // next collapse round needs that store gone to see a return-only
        // reload.  End state: `Return %carrier`, no stores, no loads, no
        // zero fill, no staging objects.
        let insts = vec![
            param_ref(2, 0),
            alloca(30, 16), // zero-prefill slot
            alloca(40, 16), // v's home
            setzero(31, 30),
            vecload(32, 30, 0),
            vecstore_i64(32, 40, 0),
            vec128_call(5, 2),
            alloca(6, 16), // carrier staging
            cast(7, 5, IrType::I128, IrType::I64),
            store(7, 6),
            bin(
                IrBinOp::LShr,
                8,
                5,
                Operand::Const(IrConst::I64(64)),
                IrType::I128,
            ),
            cast(9, 8, IrType::I128, IrType::I64),
            gep(10, 6, 8),
            store(9, 10),
            vecload(11, 6, 0),
            vecstore_i64(11, 40, 0),
            vecload(14, 40, 0),
        ];
        let mut f = func_with(insts);
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(14))));
        assert!(run(&mut f) >= 2, "forward + collapse must both fire");
        assert!(
            matches!(
                f.blocks[0].terminator,
                Terminator::Return(Some(Operand::Value(Value(5))))
            ),
            "the return must read the carrier directly: {:?}",
            f.blocks[0].terminator
        );
        let insts = &f.blocks[0].instructions;
        assert!(
            !insts.iter().any(|i| matches!(
                i,
                Instruction::Store { .. }
                    | Instruction::Alloca { .. }
                    | Instruction::Intrinsic { .. }
            )),
            "every store, staging object, zero fill and dead load must be \
             retired: {insts:?}"
        );
    }

    #[test]
    fn a_zero_fill_with_a_used_value_destination_keeps_its_object() {
        // The zero intrinsic's VALUE feeds a real consumer: deleting the
        // instruction would orphan it, so the web fails closed even though
        // the pointer write itself is unread.
        let insts = vec![
            param_ref(2, 0),
            alloca(30, 16),
            setzero(31, 30),
            // The zero VALUE escapes through a whole-vector store to the
            // caller's buffer — a live use the purge must respect.
            Instruction::Intrinsic {
                dest: None,
                op: crate::ir::intrinsics::IntrinsicOp::VecStoreI64x2,
                dest_ptr: Some(Value(2)),
                args: vec![
                    Operand::Value(Value(31)),
                    Operand::Value(Value(2)),
                    Operand::Const(IrConst::I64(0)),
                ],
            },
            vec128_call(5, 2),
            alloca(6, 16),
            cast(7, 5, IrType::I128, IrType::I64),
            store(7, 6),
            bin(
                IrBinOp::LShr,
                8,
                5,
                Operand::Const(IrConst::I64(64)),
                IrType::I128,
            ),
            cast(9, 8, IrType::I128, IrType::I64),
            gep(10, 6, 8),
            store(9, 10),
            vecload(11, 6, 0),
        ];
        let mut f = func_with(insts);
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(11))));
        assert_eq!(run(&mut f), 1, "the carrier collapse itself still fires");
        assert!(
            f.blocks[0].instructions.iter().any(|i| matches!(
                i,
                Instruction::Intrinsic {
                    op: crate::ir::intrinsics::IntrinsicOp::Setzero128,
                    ..
                }
            )),
            "a zero fill whose value is consumed must survive: {:?}",
            f.blocks[0].instructions
        );
    }

    #[test]
    fn a_carrier_reload_with_a_non_return_use_declines() {
        // The reload feeds a vector intrinsic, not a Return.  The backend's
        // vector128 return path consumes an I128 carrier directly, but its
        // intrinsic operand path does not (observed live: `movdqa %xmm15,
        // %xmm0` reading an uninitialized register), so the collapse must
        // fail closed on any non-terminator use.
        let mut f = carrier_split(&mut Vec::new());
        f.blocks[0].instructions.push(Instruction::Intrinsic {
            dest: Some(Value(12)),
            op: crate::ir::intrinsics::IntrinsicOp::VecAddI32x4,
            dest_ptr: None,
            args: vec![Operand::Value(Value(11)), Operand::Value(Value(11))],
        });
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(12))));
        assert_eq!(
            run(&mut f),
            0,
            "an intrinsic use of the reload fails closed"
        );
        assert!(f.blocks[0].instructions.iter().any(|i| matches!(
            i,
            Instruction::Intrinsic {
                op: crate::ir::intrinsics::IntrinsicOp::VecLoadI64x2,
                ..
            }
        )));
    }

    #[test]
    fn an_escaping_staging_home_keeps_its_stores() {
        // The home's address is returned alongside: the stores stay even
        // though the reload itself still collapses.
        let mut f = carrier_split(&mut Vec::new());
        f.blocks[0].instructions.push(store_ptr(6, 2));
        assert_eq!(run(&mut f), 1, "the reload itself still collapses");
        let stores = f.blocks[0]
            .instructions
            .iter()
            .filter(|i| {
                matches!(
                    i,
                    Instruction::Store {
                        ty: IrType::I64,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(stores, 2, "an escaped home's stores must survive");
    }
}
