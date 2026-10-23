//! Aggregate SROA: eliminate redundant struct/array copies.
//!
//! Struct assignment and by-value struct argument passing lower to
//! `Instruction::Memcpy` between aggregate allocas. Field reads then go
//! through `GetElementPtr` + `Load`, and a naive pipeline leaves the copies
//! in place — so a 48-byte `Particle` passed by value to an inlined
//! `particle_distance` is copied to a stack temporary before every field read
//! (gzip's `struct_copy` kernel measured 208x slower than GCC's SROA'd code).
//!
//! This pass performs three forms of scalar replacement, each provably safe
//! on non-escaping aggregate allocas:
//!
//! 1. **Load forwarding** — an alloca written only by one `Memcpy` and read
//!    only by constant-offset `Load`s has its loads redirected to the copy's
//!    source at the same offset. The copy then dies.
//!
//! 2. **Chain collapsing** — an alloca used only as the destination of one
//!    `Memcpy` and the source of others is a pure copy buffer; every
//!    `Memcpy(src=A)` is rewired to the writing copy's source and the buffer
//!    disappears.
//!
//! 3. **Copy-out expansion** — an alloca written only by constant-offset
//!    scalar stores and read only as a `Memcpy` source is split into
//!    per-field scalar allocas (which mem2reg then promotes) and each copy-out
//!    becomes per-field `Load`/`Store` pairs. Only fields the destination
//!    actually reads are materialized; unobserved tail bytes of a
//!    partially-initialized struct are not copied (LLVM SROA's "no unobserved
//!    bytes" rule). A destination that escapes (or is read with dynamic
//!    offsets) is left untouched, so the transform never has to invent
//!    residual byte copies.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrConst, IrFunction, IrModule, Operand, Value};

mod byval;
// `split`'s switches are resolved by `run_passes` (see `SplitEnv`), so the
// module has to be reachable from `crate::passes`.
pub(crate) mod split;

/// Size in bytes of a scalar IR type. Mirrors `mem2reg::promote::ir_type_size`.
fn ty_size(ty: IrType) -> i64 {
    match ty {
        IrType::I8 | IrType::U8 => 1,
        IrType::I16 | IrType::U16 => 2,
        IrType::I32 | IrType::U32 | IrType::F32 => 4,
        IrType::I64 | IrType::U64 | IrType::F64 => 8,
        IrType::D32 => 4,
        IrType::D64 => 8,
        IrType::Ptr => crate::common::types::target_ptr_size() as i64,
        IrType::I128 | IrType::U128 | IrType::F128 => 16,
        IrType::Void => 0,
    }
}

/// Instructions that can write memory the pass cannot attribute to a tracked
/// pointer: calls (including libc `memset`/`memcpy` lowered as plain `Call`s),
/// inline asm, atomics, va-machinery, and impure intrinsics.
fn is_opaque_memory_write(inst: &Instruction) -> bool {
    match inst {
        Instruction::Call { .. }
        | Instruction::CallIndirect { .. }
        | Instruction::InlineAsm { .. }
        | Instruction::AtomicRmw { .. }
        | Instruction::AtomicInc { .. }
        | Instruction::AtomicCmpxchg { .. }
        | Instruction::AtomicStore { .. }
        | Instruction::VaStart { .. }
        | Instruction::VaEnd { .. }
        | Instruction::VaCopy { .. }
        | Instruction::VaArg { .. }
        | Instruction::VaArgStruct { .. }
        | Instruction::StackRestore { .. } => true,
        Instruction::Intrinsic { op, dest_ptr, .. } => {
            !op.is_pure() || dest_ptr.is_some() || op.writes_memory_via_args()
        }
        _ => false,
    }
}

fn const_i64(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(c) => match c {
            IrConst::I64(v) => Some(*v),
            IrConst::I32(v) => Some(*v as i64),
            IrConst::I16(v) => Some(*v as i64),
            IrConst::I8(v) => Some(*v as i64),
            IrConst::Zero => Some(0),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn run(module: &mut IrModule) -> usize {
    let mut total = 0;
    for f in &mut module.functions {
        if f.is_declaration || f.blocks.is_empty() {
            continue;
        }
        let mut fn_changes = lower_non_alloca_scalar_memcpy(f);
        total += fn_changes;
        for _ in 0..6 {
            let n = run_function(f);
            if n == 0 {
                break;
            }
            fn_changes += n;
            total += n;
        }
        if fn_changes > 0 && split::sroa_trace() {
            eprintln!("[SROA] fn {} changes={}", f.name, fn_changes);
        }
    }
    total
}

/// Resolve the allocation root of a pointer expression used by a small
/// scalar memcpy. This is intentionally narrower than general alias analysis:
/// it follows only the pointer-preserving/value-offset forms emitted by the
/// frontend, and fails closed for merges or opaque producers.
///
/// Results are memoized per value in `memo` (one map per function walk; `defs`
/// and `allocas` are fixed snapshots for its lifetime). The Add/Sub arm walks
/// BOTH operands, and a shared subexpression DAG `a(i+1) = a(i) + a(i)` under
/// the pointer — reachable through an inttoptr arithmetic chain — otherwise
/// re-walks every shared node: measured 2 s at depth 25 and 52 s at depth 30
/// on a 32-line source, i.e. clean 2^n. The memo is sound because the
/// traversed def graph is the non-phi SSA graph (the walk stops at `Phi`),
/// which is acyclic; `visiting` remains as the depth/cycle fail-closed guard
/// for malformed input, and a truncated walk memoizes `None` (fail closed).
fn scalar_memcpy_root(
    value: u32,
    defs: &FxHashMap<u32, Instruction>,
    allocas: &FxHashSet<u32>,
    visiting: &mut FxHashSet<u32>,
    memo: &mut FxHashMap<u32, Option<u32>>,
) -> Option<u32> {
    if allocas.contains(&value) {
        return Some(value);
    }
    if let Some(&cached) = memo.get(&value) {
        return cached;
    }
    if visiting.len() >= 128 || !visiting.insert(value) {
        return None;
    }
    let result = match defs.get(&value) {
        Some(Instruction::GetElementPtr { base, .. }) => {
            scalar_memcpy_root(base.0, defs, allocas, visiting, memo)
        }
        Some(
            Instruction::Copy {
                src: Operand::Value(src),
                ..
            }
            | Instruction::Cast {
                src: Operand::Value(src),
                ..
            },
        ) => scalar_memcpy_root(src.0, defs, allocas, visiting, memo),
        Some(Instruction::BinOp { op, lhs, rhs, .. })
            if matches!(
                op,
                crate::ir::reexports::IrBinOp::Add | crate::ir::reexports::IrBinOp::Sub
            ) =>
        {
            let left = match lhs {
                Operand::Value(v) => scalar_memcpy_root(v.0, defs, allocas, visiting, memo),
                Operand::Const(_) => None,
            };
            let right = match rhs {
                Operand::Value(v) => scalar_memcpy_root(v.0, defs, allocas, visiting, memo),
                Operand::Const(_) => None,
            };
            match (left, right) {
                (Some(a), Some(b)) if a == b => Some(a),
                (Some(a), None) | (None, Some(a)) => Some(a),
                _ => None,
            }
        }
        // These are genuine pointer roots rather than derived aliases.
        Some(Instruction::ParamRef { .. } | Instruction::GlobalAddr { .. }) => Some(value),
        _ => None,
    };
    visiting.remove(&value);
    memo.insert(value, result);
    result
}

/// Replace small non-alloca memcpy operations with one scalar load and store.
/// This avoids materializing tiny copies as calls/spill-backed temporaries,
/// while deliberately leaving alloca-backed copies to the richer SROA forms.
///
/// Alignment: `memcpy` permits arbitrarily aligned operands, and the typed
/// scalar `Load`/`Store` pair emitted here preserves exactly that contract
/// on every backend this tree ships: x86 lowers scalar loads/stores to
/// misalignment-tolerant `mov`, AArch64 to `ldr`/`str` (tolerant for Normal
/// memory), and the RISC-V backend likewise treats unaligned accesses
/// identically to aligned ones (see its codegen README).  Overlap is not a
/// concern: overlapping `memcpy` objects are undefined behaviour in C, so
/// the load-then-store order is as sound as the call it replaces.
fn lower_non_alloca_scalar_memcpy(func: &mut IrFunction) -> usize {
    let mut allocas = FxHashSet::default();
    let mut defs = FxHashMap::default();
    let mut next = scan_max_value_id(func).saturating_add(1);
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                allocas.insert(dest.0);
            }
            if let Some(dest) = inst.dest() {
                defs.insert(dest.0, inst.clone());
            }
        }
    }
    // Shared by every memcpy root query in this function: `defs`/`allocas`
    // are immutable snapshots here, and the fresh load ids minted below are
    // never queried, so one memo serves the whole rewrite.
    let mut root_memo: FxHashMap<u32, Option<u32>> = FxHashMap::default();
    let mut changed = 0;
    for block in &mut func.blocks {
        let old = std::mem::take(&mut block.instructions);
        let old_spans = std::mem::take(&mut block.source_spans);
        let has_spans = old_spans.len() == old.len() && !old_spans.is_empty();
        let mut out = Vec::with_capacity(old.len());
        let mut spans = Vec::with_capacity(old.len());
        for (index, inst) in old.into_iter().enumerate() {
            let Some((dest, src, ty)) = (match &inst {
                Instruction::Memcpy { dest, src, size } => {
                    let ty = match size {
                        1 => Some(IrType::I8),
                        2 => Some(IrType::I16),
                        4 => Some(IrType::I32),
                        8 => Some(IrType::I64),
                        _ => None,
                    };
                    ty.and_then(|ty| {
                        let d = scalar_memcpy_root(
                            dest.0,
                            &defs,
                            &allocas,
                            &mut FxHashSet::default(),
                            &mut root_memo,
                        )?;
                        let s = scalar_memcpy_root(
                            src.0,
                            &defs,
                            &allocas,
                            &mut FxHashSet::default(),
                            &mut root_memo,
                        )?;
                        (!allocas.contains(&d) && !allocas.contains(&s))
                            .then_some((*dest, *src, ty))
                    })
                }
                _ => None,
            }) else {
                out.push(inst);
                if has_spans {
                    spans.push(old_spans[index]);
                }
                continue;
            };
            // `next_value_id` is a u32 namespace. Refuse the rewrite rather
            // than wrap or reuse an existing SSA id at the boundary.
            if next == u32::MAX {
                out.push(inst);
                if has_spans {
                    spans.push(old_spans[index]);
                }
                continue;
            }
            let load = Value(next);
            next += 1;
            out.push(Instruction::Load {
                dest: load,
                ptr: src,
                ty,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
            out.push(Instruction::Store {
                val: Operand::Value(load),
                ptr: dest,
                ty,
                seg_override: AddressSpace::Default,
                volatile: false,
            });
            if has_spans {
                spans.push(old_spans[index]);
                spans.push(old_spans[index]);
            }
            changed += 1;
        }
        block.instructions = out;
        if has_spans {
            block.source_spans = spans;
        }
    }
    if changed > 0 {
        func.next_value_id = next;
    }
    changed
}

/// A single transformation step: insert an instruction before index `at` in
/// block `block`. Applied after all in-place rewrites and removals.
#[derive(Clone)]
struct Insert {
    block: usize,
    at: usize,
    inst: Instruction,
}

struct Plan {
    // In-place pointer rewrites.
    load_ptr: Vec<(usize, usize, u32)>,
    store_ptr: Vec<(usize, usize, u32)>,
    memcpy_src: Vec<(usize, usize, u32)>,
    /// In-place value substitution: every use of the first value becomes a use
    /// of the second (operands, pointer fields and terminators alike).
    replace: Vec<(u32, u32)>,
    /// In-place call-argument substitution: (block, index, arg index, value).
    call_arg: Vec<(usize, usize, usize, u32)>,
    // Indexed removals per block.
    remove: Vec<(usize, usize)>,
    // Insertions.
    insert: Vec<Insert>,
    // Allocas to drop (dead after splitting).
    drop_allocas: FxHashSet<u32>,
    // Scalar allocas to add to the entry block.
    new_allocas: Vec<Instruction>,
}

struct Scan {
    /// Pointer derivations: value -> (base, offset). The offset is `None` for
    /// a `GetElementPtr` with a runtime offset: the value still points INTO
    /// `base`'s object, which every identity question (escape, "may this
    /// write reach the source?", "is every read of the object accounted
    /// for?") must see, even though no field can be named. See `resolve`.
    gep: FxHashMap<u32, (u32, Option<i64>)>,
    alloca_size: FxHashMap<u32, i64>,
    escapes: FxHashSet<u32>,
    memcpy: Vec<(usize, usize, u32, u32, i64)>,
    /// Library `memset` calls of a constant byte and length (see
    /// [`const_memset`]): `(block, index, ptr, byte, len, result)`. Their
    /// pointer argument is still in `escapes` -- forms 1-3 do not model a
    /// memset and must keep treating it as an opaque write -- and is left out
    /// of `split_escapes` only, because form 4 models exactly this call.
    memset: Vec<(usize, usize, u32, u8, i64, Option<Value>)>,
    /// `escapes` minus the pointer arguments of `memset` calls, or `None`
    /// when there are none (then it equals `escapes`). Read through
    /// [`Scan::split_escapes`].
    split_escapes: Option<FxHashSet<u32>>,
}

impl Scan {
    /// The escape set form 4 (`split`) uses: identical to `escapes` except
    /// that a constant library memset's pointer argument does not escape,
    /// because form 4 either rewrites that call as per-field constant stores
    /// (a whole-object memset) or refuses the object.
    fn split_escapes(&self) -> &FxHashSet<u32> {
        self.split_escapes.as_ref().unwrap_or(&self.escapes)
    }
}

/// `memset(p, c, n)` with a constant byte and a positive constant length that
/// may be treated as the library function: `(p, (unsigned char)c, n)`.
///
/// Shape and permission are the backend's own predicate
/// ([`inline_memset_const_len`](crate::backend::generation::inline_memset_const_len):
/// `CallInfo::assume_library`, so `-fno-builtin`, `-ffreestanding` and a TU
/// definition or redirect of `memset` all refuse; a constant `n`; and the
/// fortify `destlen` rule for `__memset_chk`), so the IR passes and the
/// emitter never disagree about which calls are library memsets. The SROA
/// unlock analysis in the inliner uses the same predicate, so it never
/// promises a split this module refuses.
pub(crate) fn const_memset(
    func: &str,
    info: &crate::ir::instruction::CallInfo,
) -> Option<(Value, u8, i64)> {
    let n = crate::backend::generation::inline_memset_const_len(func, info)?;
    let Some(Operand::Value(p)) = info.args.first() else {
        return None;
    };
    let Some(Operand::Const(c)) = info.args.get(1) else {
        return None;
    };
    // C11 7.24.6.1: the value is converted to `unsigned char`.
    let byte = c.to_i64()? as u8;
    (n > 0).then_some((*p, byte, n as i64))
}

fn scan(func: &IrFunction) -> Scan {
    let mut s = Scan {
        gep: FxHashMap::default(),
        alloca_size: FxHashMap::default(),
        escapes: FxHashSet::default(),
        memcpy: Vec::new(),
        memset: Vec::new(),
        split_escapes: None,
    };
    // Pointer arguments of constant memsets; merged into `escapes` at the end
    // (see `Scan::memset`).
    let mut memset_ptrs: Vec<u32> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::Alloca { dest, size, .. } => {
                    s.alloca_size.insert(dest.0, *size as i64);
                }
                // Runtime-offset GEPs are recorded too (offset `None`).
                // Skipping them made `&s.v[k]` look like an unrelated object:
                // a store through it did not dirty a forwarded copy's source,
                // a load through it was not counted as a read of the object,
                // and passing it to a call did not make the object escape.
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    s.gep.insert(dest.0, (base.0, const_i64(offset)));
                }
                // A `Copy` of a pointer is a zero-offset alias. Following it
                // keeps aggregate copies visible when the frontend routes the
                // struct pointer through a temporary (e.g. `tmp = sret;
                // memcpy(dst, tmp)`), which otherwise defeats copy-buffer
                // detection and store-only analysis.
                Instruction::Copy {
                    dest,
                    src: Operand::Value(v),
                } => {
                    s.gep.insert(dest.0, (v.0, Some(0)));
                }
                Instruction::Memcpy { dest, src, size } => {
                    s.memcpy.push((bi, ii, dest.0, src.0, *size as i64));
                }
                Instruction::Store {
                    val: Operand::Value(v),
                    ..
                } => {
                    s.escapes.insert(v.0); // a pointer stored to memory escapes
                }
                _ => {}
            }
        }
        if let crate::ir::reexports::Terminator::Return(Some(Operand::Value(v))) = &block.terminator
        {
            s.escapes.insert(v.0);
        }
    }
    // Second sweep: mark every value reachable through an instruction this
    // pass does not model as ESCAPING.
    //
    // This is deliberately an ALLOWLIST. The original version listed only
    // `Call`, `CallIndirect` and `Phi`, so any other consumer of an aggregate
    // pointer was silently treated as "no use at all". `Instruction::Intrinsic`
    // takes its aggregate operands directly (SIMD builtins pass a 16-byte
    // vector by pointer, e.g. `SubPs128 [Value(13), Value(20)]`), so a memcpy
    // destination feeding an intrinsic looked dead, the copy was deleted, and
    // codegen aborted with "value 13 has no register, stack slot, or Copy
    // definition" on four SIMD regression tests.
    //
    // Anything not explicitly understood below conservatively escapes; a new
    // IR instruction can therefore never silently defeat the analysis.
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                // Understood and handled by the transforms themselves.
                Instruction::Alloca { .. }
                | Instruction::GetElementPtr { .. }
                | Instruction::Copy { .. }
                | Instruction::Memcpy { .. }
                | Instruction::Load { .. }
                | Instruction::Store { .. }
                | Instruction::PgoCounterInc { .. } => {}

                // A constant library memset: recorded for form 4. Its other
                // two arguments are constants, so the pointer is its only
                // value operand.
                Instruction::Call { func: name, info } if const_memset(name, info).is_some() => {
                    if let Some((p, byte, len)) = const_memset(name, info) {
                        s.memset.push((bi, ii, p.0, byte, len, info.dest));
                        memset_ptrs.push(p.0);
                    }
                }
                // Arguments to a call leave the function.
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    for a in &info.args {
                        if let Operand::Value(v) = a {
                            s.escapes.insert(v.0);
                        }
                    }
                }

                // A phi merges pointers along edges this pass does not track.
                Instruction::Phi { incoming, .. } => {
                    for (v, _) in incoming {
                        if let Operand::Value(v) = v {
                            s.escapes.insert(v.0);
                        }
                    }
                }

                // Everything else: every value operand AND every pointer field
                // escapes. Covers Intrinsic (incl. dest_ptr), InlineAsm, the
                // va_* family, atomics, Select, and any future variant.
                other => {
                    let mut esc = other.clone();
                    esc.for_each_operand_mut(|op| {
                        if let Operand::Value(v) = op {
                            s.escapes.insert(v.0);
                        }
                    });
                    // Bare-Value positions (va_list ptrs, StackRestore,
                    // InlineAsm outputs, ...) escape too; the exhaustive
                    // walker covers them all (a manual Intrinsic-only check
                    // missed the rest, understating escapes).
                    esc.for_each_value_use_mut(|v| {
                        s.escapes.insert(v.0);
                    });
                }
            }
        }
    }
    if !memset_ptrs.is_empty() {
        s.split_escapes = Some(s.escapes.clone());
        s.escapes.extend(memset_ptrs);
    }
    s
}

/// Branch targets of a block's terminator. Used to reject blocks that a back
/// edge can re-enter, where a single-pass store/copy-out shape is not valid.
fn block_targets(b: &crate::ir::reexports::BasicBlock) -> Vec<usize> {
    use crate::ir::reexports::Terminator;
    match &b.terminator {
        Terminator::Branch(t) => vec![t.0 as usize],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            vec![true_label.0 as usize, false_label.0 as usize]
        }
        Terminator::Switch { default, cases, .. } => {
            let mut v = vec![default.0 as usize];
            v.extend(cases.iter().map(|(_, t)| t.0 as usize));
            v
        }
        _ => Vec::new(),
    }
}

/// Walk a pointer value to the object it is derived from.
///
/// Returns `(root, offset)`. The offset is `None` when any step of the chain
/// has a runtime offset: the pointer is derived from `root`, so identity
/// questions must treat it as `root`, but it names no particular field and
/// nothing may be forwarded or split through it.
fn resolve(gep: &FxHashMap<u32, (u32, Option<i64>)>, mut v: u32) -> (u32, Option<i64>) {
    let mut off = Some(0i64);
    while let Some(&(b, o)) = gep.get(&v) {
        off = match (off, o) {
            (Some(acc), Some(step)) => Some(acc.saturating_add(step)),
            _ => None,
        };
        v = b;
    }
    (v, off)
}

/// True if the alloca `target` (or any GEP/copy-alias chain rooted at it)
/// escapes — its address leaves the function via a store, a call, a return,
/// or a Phi. Any such path makes every scalar-replacement transform unsound.
fn any_escape(
    escapes: &FxHashSet<u32>,
    gep: &FxHashMap<u32, (u32, Option<i64>)>,
    target: u32,
) -> bool {
    if escapes.contains(&target) {
        return true;
    }
    escapes.iter().any(|&v| resolve(gep, v).0 == target)
}

/// Iterate every pointer operand of every instruction and classify the use of
/// each referenced value. Used to prove an alloca is only used as a memcpy
/// buffer (dest or src) — never loaded/stored/GEP'd/escaped.
#[derive(Clone)]
enum PtrUse {
    MemcpyDest,
    MemcpySrc,
    Other,
}

fn pointer_uses(
    func: &IrFunction,
    gep: &FxHashMap<u32, (u32, Option<i64>)>,
) -> FxHashMap<u32, Vec<PtrUse>> {
    let mut m: FxHashMap<u32, Vec<PtrUse>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr { base, .. } => {
                    let (r, _) = resolve(gep, base.0);
                    m.entry(r).or_default().push(PtrUse::Other);
                }
                Instruction::Load { ptr, .. } => {
                    let (r, _) = resolve(gep, ptr.0);
                    m.entry(r).or_default().push(PtrUse::Other);
                }
                Instruction::Store { ptr, .. } => {
                    let (r, _) = resolve(gep, ptr.0);
                    m.entry(r).or_default().push(PtrUse::Other);
                }
                Instruction::Memcpy { dest, src, .. } => {
                    let (dr, _) = resolve(gep, dest.0);
                    let (sr, _) = resolve(gep, src.0);
                    m.entry(dr).or_default().push(PtrUse::MemcpyDest);
                    m.entry(sr).or_default().push(PtrUse::MemcpySrc);
                }
                _ => {}
            }
        }
    }
    m
}

/// Compute the true maximum value id referenced anywhere in the function
/// (destinations AND operands), without trusting the `next_value_id` cache,
/// which can under-report after passes that insert values without bumping it.
fn scan_max_value_id(func: &IrFunction) -> u32 {
    let mut max_id = 0u32;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(v) = inst.dest() {
                max_id = max_id.max(v.0);
            }
            for v in inst.used_values() {
                max_id = max_id.max(v);
            }
        }
        for v in block.terminator.used_values() {
            max_id = max_id.max(v);
        }
    }
    max_id
}

fn run_function(func: &mut IrFunction) -> usize {
    let s = scan(func);
    let mut next = scan_max_value_id(func).saturating_add(1);
    let mut plan = Plan {
        load_ptr: Vec::new(),
        store_ptr: Vec::new(),
        memcpy_src: Vec::new(),
        replace: Vec::new(),
        call_arg: Vec::new(),
        remove: Vec::new(),
        insert: Vec::new(),
        drop_allocas: FxHashSet::default(),
        new_allocas: Vec::new(),
    };
    let mut changed = 0usize;

    // ── Analyze writes per alloca ──────────────────────────────────────────
    let mut memcpy_writer: FxHashMap<u32, (usize, usize, u32, i64)> = FxHashMap::default(); // alloca -> (block, idx, src, size)
    let mut memcpy_writer_count: FxHashMap<u32, u32> = FxHashMap::default();
    let mut store_target: FxHashSet<u32> = FxHashSet::default();
    let mut store_fields: FxHashMap<u32, Vec<(i64, IrType)>> = FxHashMap::default();
    for &(bi, ii, d, src, size) in &s.memcpy {
        let (r, off) = resolve(&s.gep, d);
        if !s.alloca_size.contains_key(&r) {
            continue;
        }
        // EVERY copy into the root counts as a writer, at any offset: a
        // member assignment `d.in = n` lowers to `Memcpy GEP(d, 8) <- n`.
        // Counting only offset-0 copies made `d = s; d.in = n; d.in.x`
        // look singly written, and Transform 1 forwarded the member read
        // to `s.in.x`, past the copy that replaced it.
        *memcpy_writer_count.entry(r).or_insert(0) += 1;
        if off == Some(0) {
            memcpy_writer.insert(r, (bi, ii, src, size));
        }
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Store { ptr, ty, .. } = inst {
                let (r, off) = resolve(&s.gep, ptr.0);
                store_target.insert(r);
                if let Some(off) = off
                    && off >= 0
                    && s.alloca_size.contains_key(&r)
                {
                    store_fields.entry(r).or_default().push((off, *ty));
                }
            }
        }
    }

    // ── Load-reads per value (relative offsets) ────────────────────────────
    let mut load_reads: FxHashMap<u32, Vec<(i64, IrType)>> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Load { ptr, ty, .. } = inst {
                let (r, off) = resolve(&s.gep, ptr.0);
                if let Some(off) = off {
                    load_reads.entry(r).or_default().push((off, *ty));
                }
            }
        }
    }

    // ── 1. Forward loads through a memcpy-written alloca ───────────────────
    // The alloca must be written exactly once by a Memcpy (full, offset 0),
    // never stored-to, never escaped, and the load must read a constant
    // offset inside the copied size. The Memcpy must dominate the load within
    // the same block and nothing may modify the copy SOURCE in between.
    //
    // When EVERY read of such an alloca is forwarded, the writing Memcpy is
    // dead and is removed explicitly (a generic "dest unused" sweep is
    // unsound: dynamic GEPs into a shared alloca look unused while the same
    // bytes are still read through a different GEP value).
    // EVERY load of the object counts, whatever its offset: the dead-writer
    // removal below deletes the copy once `forwarded == loads`, so a read that
    // is not counted here (a runtime index, an offset outside the copy) would
    // be left reading an object whose initializing copy is gone.
    let mut loads_from: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Load { ptr, .. } = inst {
                let (r, _) = resolve(&s.gep, ptr.0);
                *loads_from.entry(r).or_insert(0) += 1;
            }
        }
    }
    let mut forwarded_from: FxHashMap<u32, u32> = FxHashMap::default();
    // Roots that Transform 1's plan will make a forwarded LOAD read. The
    // plan inserts a new GEP rooted at the writer's `src`; after the rewrite
    // that root is load-reachable, so its own initializing memcpy must
    // survive. Both the dead-writer removal below and the copy-buffer
    // collapse after it key on the PRE-plan instruction stream and would
    // otherwise see the root as memcpy-only and delete its writer, leaving
    // the forwarded load reading an uninitialized slot (observed:
    // linux-6.18 pte_mkwrite returned leftover stack as the PTE because
    // `Memcpy V67 <- V0` (an inlined by-value pte_t argument bridge) was
    // collapsed away after another load had just been forwarded onto V67).
    let mut forward_target_roots: crate::common::fx_hash::FxHashSet<u32> =
        crate::common::fx_hash::FxHashSet::default();
    {
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                if let Instruction::Load { ptr, ty, .. } = inst {
                    let (r, off) = resolve(&s.gep, ptr.0);
                    if let Some(&(mb, mi, src, size)) = memcpy_writer.get(&r) {
                        // A runtime-offset read names no field of the source.
                        let Some(off) = off else {
                            continue;
                        };
                        if memcpy_writer_count.get(&r).copied().unwrap_or(0) != 1
                            || store_target.contains(&r)
                            || s.escapes.contains(&r)
                            || off < 0
                            || off + ty_size(*ty) > size
                        {
                            continue;
                        }
                        if mb != bi || mi >= ii {
                            continue; // memcpy must precede the load in this block
                        }
                        // The source must not be written between the copy and
                        // the load (aliasing safety).
                        let (sr, _so) = resolve(&s.gep, src);
                        // A non-escaping alloca cannot be written by a call or
                        // other opaque instruction; any other source (param,
                        // global, escaped alloca) must be assumed clobbered by
                        // one. libc memset/memcpy lower to plain Calls: SQLite
                        // memjrnlCreateFile's `copy = *p; memset(p,0,..)` was
                        // forwarded across the memset by the old Store/Memcpy-
                        // only scan (speedtest1 --testset json SIGSEGV).
                        let src_call_private =
                            s.alloca_size.contains_key(&sr) && !any_escape(&s.escapes, &s.gep, sr);
                        // A store whose pointer value differs from the source
                        // is only proven no-alias when the source is a *private*
                        // (non-escaping) alloca reachable only through its own
                        // value. Otherwise two distinct pointer values (e.g. two
                        // function parameters passed the same address, as in
                        //   u64 bits; memcpy(&bits,p,8); *q=0; memcpy(&d,&bits,8);
                        // with p==q) may still alias, and forwarding must not
                        // read the source after the write. This is the
                        // memcpy-rewrite soundness hole exposed by the
                        // constant-size __builtin_memcpy lowering (2026-09-09).
                        // Prove whether an intervening memory write may modify the
                        // copy source. A write to a *private* (non-escaping)
                        // alloca cannot touch any other object, and a private
                        // alloca source cannot be touched by a write through any
                        // other pointer value. Otherwise two distinct pointer
                        // values (e.g. two params passed the same address) may
                        // still alias, so the source must be considered dirty.
                        let is_private = |root: u32| -> bool {
                            s.alloca_size.contains_key(&root)
                                && !any_escape(&s.escapes, &s.gep, root)
                        };
                        let may_write_source = |pr: u32| -> bool {
                            pr == sr                // same value (or same GEP root)
                                || !(src_call_private || is_private(pr))
                        };
                        let mut src_dirty = false;
                        for k in (mi + 1)..ii {
                            if let Some(inst_k) = block.instructions.get(k) {
                                match inst_k {
                                    Instruction::Store { ptr, .. } => {
                                        let (pr, _) = resolve(&s.gep, ptr.0);
                                        if may_write_source(pr) {
                                            src_dirty = true;
                                            break;
                                        }
                                    }
                                    Instruction::Memcpy { dest, .. } => {
                                        let (pr, _) = resolve(&s.gep, dest.0);
                                        if may_write_source(pr) {
                                            src_dirty = true;
                                            break;
                                        }
                                    }
                                    other => {
                                        if !src_call_private && is_opaque_memory_write(other) {
                                            src_dirty = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        if src_dirty {
                            continue;
                        }
                        let d = next;
                        next += 1;
                        plan.load_ptr.push((bi, ii, d));
                        let (fwd_root, _) = resolve(&s.gep, src);
                        forward_target_roots.insert(fwd_root);
                        plan.insert.push(Insert {
                            block: bi,
                            at: ii,
                            inst: Instruction::GetElementPtr {
                                dest: Value(d),
                                base: Value(src),
                                offset: Operand::Const(IrConst::ptr_int(off)),
                                ty: IrType::Ptr,
                            },
                        });
                        *forwarded_from.entry(r).or_insert(0) += 1;
                        changed += 1;
                    }
                }
            }
        }
    }
    // Remove a memcpy whose alloca is now read only through forwarded loads.
    {
        let uses = pointer_uses(func, &s.gep);
        for (&r, &(mb, mi, _src, _size)) in &memcpy_writer {
            if memcpy_writer_count.get(&r).copied().unwrap_or(0) != 1 {
                continue;
            }
            if store_target.contains(&r) || any_escape(&s.escapes, &s.gep, r) {
                continue;
            }
            // A forwarded load from another buffer was just re-pointed at
            // this buffer's content (its writer's src): removing THIS writer
            // would strand that load on an uninitialized slot.
            if forward_target_roots.contains(&r) {
                continue;
            }
            let n_loads = loads_from.get(&r).copied().unwrap_or(0);
            if n_loads == 0 || forwarded_from.get(&r).copied().unwrap_or(0) != n_loads {
                continue;
            }
            let u = uses.get(&r).cloned().unwrap_or_default();
            let n_dest = u.iter().filter(|u| matches!(u, PtrUse::MemcpyDest)).count();
            let n_src = u.iter().filter(|u| matches!(u, PtrUse::MemcpySrc)).count();
            // Every read of r was forwarded (forwarded == loads), r is not
            // stored-to/escaped, written once by this memcpy, and read by no
            // other memcpy. The remaining "Other" uses are GEP bases feeding
            // those (now dead) loads, so the writer is dead.
            if n_dest == 1 && n_src == 0 {
                plan.remove.push((mb, mi));
                changed += 1;
            }
        }
    }

    // ── 2. Collapse copy-buffer allocas ────────────────────────────────────
    {
        let uses = pointer_uses(func, &s.gep);
        for (&r, &(mb, mi, w_src, _)) in &memcpy_writer {
            if memcpy_writer_count.get(&r).copied().unwrap_or(0) != 1 {
                continue;
            }
            if store_target.contains(&r) || any_escape(&s.escapes, &s.gep, r) {
                continue;
            }
            // pointer_uses() sees the PRE-plan stream: uses Transform 1 is
            // about to create (forwarded loads re-pointed at this buffer)
            // are invisible here, so "all uses are memcpy uses" can be
            // false by the time the plan applies. Never collapse a buffer
            // that a forwarded load will read.
            if forward_target_roots.contains(&r) {
                continue;
            }
            let u = uses.get(&r).cloned().unwrap_or_default();
            let n_dest = u.iter().filter(|u| matches!(u, PtrUse::MemcpyDest)).count();
            let n_src = u.iter().filter(|u| matches!(u, PtrUse::MemcpySrc)).count();
            // All uses are memcpy uses; exactly one dest (the writer) and >=1 src.
            if n_dest != 1 || n_src == 0 || (n_dest + n_src) != u.len() {
                continue;
            }
            // Rewire every reader's src to the writer's src.
            //
            // SOUNDNESS: reading from the writer's source instead of from the
            // buffer is only valid while the two still hold the same bytes.
            // Transform 1 proves that with an ordering + clobber check; this
            // transform originally rewired unconditionally, so
            //
            //     memcpy(r, SRC, n);   ... store into SRC ...   memcpy(dst, r, n)
            //
            // silently started copying the NEW contents of SRC. That corrupted
            // a loop bound in structs_bitfields and the program never
            // terminated. Apply the same two proofs here:
            //   * the writing memcpy must precede the reader in the SAME block
            //     (no dominance information is available in this pass, and a
            //     cross-block reader may be reached without the writer), and
            //   * neither the buffer nor the source may be written in between.
            let (w_src_root, _) = resolve(&s.gep, w_src);
            let mut rewired = 0;
            let mut unsafe_reader = false;
            let mut pending: Vec<(usize, usize)> = Vec::new();
            for (bi, block) in func.blocks.iter().enumerate() {
                for (ii, inst) in block.instructions.iter().enumerate() {
                    if let Instruction::Memcpy { src, .. } = inst {
                        let (sr, soff) = resolve(&s.gep, src.0);
                        if sr != r || soff != Some(0) {
                            continue;
                        }
                        if bi != mb || ii <= mi {
                            unsafe_reader = true; // not dominated by the writer
                            break;
                        }
                        let mut dirty = false;
                        for k in (mi + 1)..ii {
                            match block.instructions.get(k) {
                                Some(Instruction::Store { ptr, .. }) => {
                                    let (pr, _) = resolve(&s.gep, ptr.0);
                                    if pr == w_src_root || pr == r {
                                        dirty = true;
                                        break;
                                    }
                                }
                                Some(Instruction::Memcpy { dest, .. }) => {
                                    let (pr, _) = resolve(&s.gep, dest.0);
                                    if pr == w_src_root || pr == r {
                                        dirty = true;
                                        break;
                                    }
                                }
                                // Anything that can write memory opaquely may
                                // touch either pointer (calls, inline asm,
                                // atomics, impure intrinsics, va machinery).
                                Some(other) if is_opaque_memory_write(other) => {
                                    dirty = true;
                                    break;
                                }
                                _ => {}
                            }
                        }
                        if dirty {
                            unsafe_reader = true;
                            break;
                        }
                        pending.push((bi, ii));
                    }
                }
                if unsafe_reader {
                    break;
                }
            }
            // All-or-nothing: a single unprovable reader leaves the buffer
            // alone, otherwise the copy would be deleted while that reader
            // still needs it.
            if unsafe_reader {
                continue;
            }
            for (bi, ii) in pending {
                plan.memcpy_src.push((bi, ii, w_src));
                rewired += 1;
                changed += 1;
            }
            if rewired > 0 {
                // The writer is now the only user of r; drop it.
                plan.remove.push((mb, mi));
                plan.drop_allocas.insert(r);
                changed += 1;
            }
        }
    }

    // ── 3. Copy-out expansion for store-only allocas ───────────────────────
    //
    // DISABLED BY DEFAULT (opt in with CCC_SROA_COPYOUT=1).
    //
    // Splitting a store-only buffer into per-field scalar allocas is only
    // valid when every field store provably reaches every copy-out with the
    // value that copy-out must observe. Proving that needs a dominator tree
    // and loop information this pass does not build: the scalar allocas live
    // for the whole function, while the buffer is rewritten per iteration, so
    // in a loop the expansion happily delivers a stale field. Measured: it
    // hangs structs_bitfields and simd_sse_float (one transform, one hang).
    //
    // The local shape checks below (single home block, all stores before the
    // first reader, no re-entry) are necessary but demonstrably NOT
    // sufficient, so the transform stays off rather than shipping a guard
    // that only appears to work. Transforms 1 and 2 already deliver the
    // struct-copy win and are on by default.
    if std::env::var("CCC_SROA_COPYOUT").is_ok() {
        // Objects reached through a runtime offset anywhere: their accesses
        // cannot be mapped onto fields, so the form refuses them outright.
        let unknown_offset_roots: FxHashSet<u32> = s
            .gep
            .keys()
            .filter_map(|&v| match resolve(&s.gep, v) {
                (root, None) => Some(root),
                _ => None,
            })
            .collect();
        for (&r, fields) in &store_fields {
            if unknown_offset_roots.contains(&r) {
                continue;
            }
            if memcpy_writer_count.get(&r).copied().unwrap_or(0) > 0 {
                continue; // also written by a memcpy — not a pure store-only source
            }
            if s.escapes.contains(&r) {
                continue;
            }
            if load_reads.get(&r).map(|v| !v.is_empty()).unwrap_or(false) {
                continue; // read directly — not a pure memcpy source
            }
            let size = *s.alloca_size.get(&r).unwrap_or(&0);
            if size <= 0 {
                continue;
            }
            // Enumerate the memcpys that read r as their full source.
            let mut readers: Vec<(usize, usize, u32)> = Vec::new();
            for (bi, block) in func.blocks.iter().enumerate() {
                for (ii, inst) in block.instructions.iter().enumerate() {
                    if let Instruction::Memcpy { dest, src, .. } = inst {
                        let (sr, soff) = resolve(&s.gep, src.0);
                        if sr == r && soff == Some(0) {
                            readers.push((bi, ii, dest.0));
                        }
                    }
                }
            }
            if readers.is_empty() {
                continue;
            }

            // ORDERING PROOF. Replacing the buffer with per-field scalars is
            // only valid if every field store that feeds a copy-out actually
            // executes before it, exactly once. Without this the expansion
            // reads a scalar alloca that has not been written yet (or was
            // written by a previous loop iteration): simd_sse_float and
            // structs_bitfields both hung, because a value the copy-out was
            // supposed to deliver never arrived.
            //
            // This pass has no dominator tree, so require the conservative
            // shape it can verify locally: every field store AND every reader
            // live in ONE block, each store precedes every reader there, and
            // the block is not part of a cycle (no back edge into it), so the
            // sequence executes at most once per call. Anything else is left
            // to a future dominance-aware version rather than guessed at.
            let mut home: Option<usize> = None;
            let mut ok_shape = true;
            for (bi, block) in func.blocks.iter().enumerate() {
                for inst in &block.instructions {
                    if let Instruction::Store { ptr, .. } = inst {
                        let (sr, _) = resolve(&s.gep, ptr.0);
                        if sr == r {
                            match home {
                                None => home = Some(bi),
                                Some(h) if h == bi => {}
                                _ => {
                                    ok_shape = false;
                                }
                            }
                        }
                    }
                }
            }
            let home = match home {
                Some(h) if ok_shape => h,
                _ => continue,
            };
            if readers.iter().any(|&(bi, _, _)| bi != home) {
                continue;
            }
            // Last store must precede the first reader.
            let first_reader = readers.iter().map(|&(_, ii, _)| ii).min().unwrap_or(0);
            let mut last_store = 0usize;
            for (ii, inst) in func.blocks[home].instructions.iter().enumerate() {
                if let Instruction::Store { ptr, .. } = inst {
                    let (sr, _) = resolve(&s.gep, ptr.0);
                    if sr == r {
                        last_store = last_store.max(ii);
                    }
                }
            }
            if last_store >= first_reader {
                continue;
            }
            // The block must not be re-entered (a back edge would carry the
            // scalar allocas across iterations while the buffer would have
            // been rewritten).
            // Any predecessor at or after `home` in layout order implies a
            // back edge into it. (An earlier `bi >= home` guard also demanded
            // the predecessor be the block itself or later, which let a loop
            // whose latch precedes the body slip through.)
            let mut pred_count = 0usize;
            let mut has_back_edge = false;
            for (bi, b) in func.blocks.iter().enumerate() {
                if block_targets(b).into_iter().any(|t| t == home) {
                    pred_count += 1;
                    if bi >= home {
                        has_back_edge = true;
                    }
                }
            }
            let reentrant = has_back_edge || pred_count > 1;
            if reentrant {
                continue;
            }
            // Pre-validate: EVERY reader must be safely expandable, and each
            // must actually read at least one field (a zero-read destination
            // is a copy buffer — left to chain collapsing so no data is
            // silently dropped). All-or-nothing: if any reader fails, the
            // source alloca is left untouched.
            let mut expandable = !readers.is_empty();
            for &(_, _, d) in &readers {
                let (dr, _) = resolve(&s.gep, d);
                if s.escapes.contains(&d)
                    || s.escapes.contains(&dr)
                    || unknown_offset_roots.contains(&dr)
                {
                    expandable = false;
                    break;
                }
                let reads = load_reads.get(&d).cloned().unwrap_or_default();
                if reads.is_empty() || reads.iter().any(|&(o, _)| o < 0) {
                    expandable = false;
                    break;
                }
            }
            if !expandable {
                continue;
            }

            // Dedup field offsets.
            let mut unique: Vec<(i64, IrType)> = Vec::new();
            for &(off, ty) in fields {
                if !unique.iter().any(|&(o, _)| o == off) {
                    unique.push((off, ty));
                }
            }
            unique.sort_by_key(|&(o, _)| o);
            // Create one scalar alloca per field.
            let mut field_allocas: Vec<(i64, IrType, u32)> = Vec::new();
            for &(off, ty) in &unique {
                let a = next;
                next += 1;
                field_allocas.push((off, ty, a));
                plan.new_allocas.push(Instruction::Alloca {
                    dest: Value(a),
                    ty,
                    size: ty_size(ty) as usize,
                    align: 0,
                    volatile: false,
                    semantic_volatile: false,
                });
            }
            // Rewrite the source's field stores to the scalar allocas.
            for (bi, block) in func.blocks.iter().enumerate() {
                for (ii, inst) in block.instructions.iter().enumerate() {
                    if let Instruction::Store { ptr, .. } = inst {
                        let (sr, soff) = resolve(&s.gep, ptr.0);
                        if sr == r {
                            if let Some(&(_, _, a)) =
                                field_allocas.iter().find(|&&(o, _, _)| Some(o) == soff)
                            {
                                plan.store_ptr.push((bi, ii, a));
                            }
                        }
                    }
                }
            }
            // Expand each reader into per-field copies, limited to the reads.
            for (bi, ii, d) in readers {
                let reads = load_reads.get(&d).cloned().unwrap_or_default();
                let mut new_insts: Vec<Instruction> = Vec::new();
                for &(off, ty, a) in &field_allocas {
                    let needed = reads
                        .iter()
                        .any(|&(ro, rt)| off < ro + ty_size(rt) && ro < off + ty_size(ty));
                    if !needed {
                        continue;
                    }
                    let ld = next;
                    next += 1;
                    new_insts.push(Instruction::Load {
                        volatile: false,
                        dest: Value(ld),
                        ptr: Value(a),
                        ty,
                        seg_override: AddressSpace::Default,
                    });
                    let gd = next;
                    next += 1;
                    new_insts.push(Instruction::GetElementPtr {
                        dest: Value(gd),
                        base: Value(d),
                        offset: Operand::Const(IrConst::ptr_int(off)),
                        ty: IrType::Ptr,
                    });
                    new_insts.push(Instruction::Store {
                        volatile: false,
                        val: Operand::Value(Value(ld)),
                        ptr: Value(gd),
                        ty,
                        seg_override: AddressSpace::Default,
                    });
                }
                plan.remove.push((bi, ii));
                // Insert the field copies where the memcpy was.
                for (k, ins) in new_insts.into_iter().enumerate() {
                    plan.insert.push(Insert {
                        block: bi,
                        at: ii + k,
                        inst: ins,
                    });
                }
                changed += 1;
            }
            // The aggregate alloca r is now dead.
            plan.drop_allocas.insert(r);
        }
    }

    // ── 5. Build a by-value aggregate argument at its call site ────────────
    // Runs before the split because it *creates* the split opportunity: it
    // retires the object a `Memcpy` named, which is the one shape form 4 has to
    // refuse. Everything it plans is claimed for the whole iteration, so form 4
    // sees the copy removed from `func` and both objects already accounted for;
    // the field allocas it adds are consumed by mem2reg downstream.
    changed += byval::run_function(func, &s, &mut plan, &mut next);

    // ── 4. Split constant-offset aggregates into per-field allocas ─────────
    // Runs last so forms 1-3 get first refusal on any object they can express
    // as a copy; `split::run_function` refuses everything they claimed, so no
    // instruction is ever planned twice.
    changed += split::run_function(func, &s, &mut plan, &mut next);

    // ── Apply the plan ──────────────────────────────────────────────────────
    for &(bi, ii, v) in &plan.load_ptr {
        if let Instruction::Load { ptr, .. } = &mut func.blocks[bi].instructions[ii] {
            *ptr = Value(v);
        }
    }
    for &(bi, ii, v) in &plan.store_ptr {
        if let Instruction::Store { ptr, .. } = &mut func.blocks[bi].instructions[ii] {
            *ptr = Value(v);
        }
    }
    for &(bi, ii, v) in &plan.memcpy_src {
        if let Instruction::Memcpy { src, .. } = &mut func.blocks[bi].instructions[ii] {
            *src = Value(v);
        }
    }
    // Value substitution (object coalescing). Ordered before the block rebuild
    // because it addresses original positions, and applied to terminators as
    // well so a value that reaches control flow is not left dangling.
    for &(from, to) in &plan.replace {
        fn sub_op(op: &mut Operand, from: u32, to: u32) {
            if let Operand::Value(v) = op {
                if v.0 == from {
                    *v = Value(to);
                }
            }
        }
        fn sub_val(v: &mut Value, from: u32, to: u32) {
            if v.0 == from {
                *v = Value(to);
            }
        }
        let mut op = |op: &mut Operand| sub_op(op, from, to);
        let mut val = |v: &mut Value| sub_val(v, from, to);
        for block in func.blocks.iter_mut() {
            block.terminator.for_each_operand_mut(&mut op);
            for inst in block.instructions.iter_mut() {
                inst.for_each_operand_mut(&mut op);
                inst.for_each_value_use_mut(&mut val);
            }
        }
    }
    // Call-argument substitution (a by-value image fed from the coalesced
    // object). Also positional, so it shares the pre-rebuild phase.
    for &(bi, ii, ai, v) in &plan.call_arg {
        if bi >= func.blocks.len() {
            continue;
        }
        if let Some(Instruction::Call { info, .. }) = func.blocks[bi].instructions.get_mut(ii) {
            if let Some(arg) = info.args.get_mut(ai) {
                *arg = Operand::Value(Value(v));
            }
        }
    }
    // Apply removals and insertions TOGETHER, in one rebuild per block.
    //
    // Doing them in two phases is wrong: `plan.remove` and `plan.insert` both
    // index the ORIGINAL instruction list, so deleting first shifts every
    // later position down and each insert then lands one slot too late. That
    // put a freshly created `GetElementPtr` AFTER the `Load` that consumes it,
    // producing IR that violates SSA def-before-use:
    //
    //   Load        { dest: 51, ptr: Value(73) }        <- use
    //   GetElementPtr { dest: Value(73), base: 76, +8 } <- def, too late
    //
    // The backend faithfully emits that order (`movsd (%r11),%xmm5` before
    // `leaq 8(%rdx),%r11`), so the program read an undefined register and
    // struct_copy segfaulted. This was previously misattributed to a backend
    // scheduling defect; the backend is innocent.
    //
    // Rebuilding the block in a single pass keeps every index relative to the
    // original list, so an insert "at i" is always placed immediately before
    // original instruction i regardless of what else was removed.
    {
        let mut ins_by_block: FxHashMap<usize, Vec<(usize, Instruction)>> = FxHashMap::default();
        for ins in plan.insert {
            ins_by_block
                .entry(ins.block)
                .or_default()
                .push((ins.at, ins.inst));
        }
        let mut rm_by_block: FxHashMap<usize, FxHashSet<usize>> = FxHashMap::default();
        for &(bi, ii) in &plan.remove {
            rm_by_block.entry(bi).or_default().insert(ii);
        }

        let touched: FxHashSet<usize> = ins_by_block
            .keys()
            .chain(rm_by_block.keys())
            .copied()
            .collect();
        for bi in touched {
            if bi >= func.blocks.len() {
                continue;
            }
            let mut inserts = ins_by_block.remove(&bi).unwrap_or_default();
            // Stable by position: several inserts at the same index keep the
            // order the planner produced (a GEP before the load that uses it).
            inserts.sort_by_key(|&(at, _)| at);
            let removes = rm_by_block.remove(&bi).unwrap_or_default();

            let block = &mut func.blocks[bi];
            let old = std::mem::take(&mut block.instructions);
            let old_spans = std::mem::take(&mut block.source_spans);
            let has_spans = old_spans.len() == old.len() && !old_spans.is_empty();
            let fill = old_spans
                .last()
                .copied()
                .unwrap_or_else(crate::common::source::Span::dummy);

            let mut out = Vec::with_capacity(old.len() + inserts.len());
            let mut spans = Vec::with_capacity(out.capacity());
            let mut next_ins = 0usize;
            for (i, inst) in old.into_iter().enumerate() {
                while next_ins < inserts.len() && inserts[next_ins].0 <= i {
                    out.push(inserts[next_ins].1.clone());
                    spans.push(if has_spans { old_spans[i] } else { fill });
                    next_ins += 1;
                }
                if !removes.contains(&i) {
                    out.push(inst);
                    spans.push(if has_spans { old_spans[i] } else { fill });
                }
            }
            // Anything anchored past the end appends in planner order.
            while next_ins < inserts.len() {
                out.push(inserts[next_ins].1.clone());
                spans.push(fill);
                next_ins += 1;
            }
            block.instructions = out;
            block.source_spans = if has_spans { spans } else { Vec::new() };
        }
    }
    // Add scalar field allocas to the entry block.
    if !plan.new_allocas.is_empty() {
        for ins in plan.new_allocas.into_iter().rev() {
            func.blocks[0].instructions.insert(0, ins);
        }
    }

    // Drop dead aggregate allocas -- AFTER the rebuild above.
    //
    // Two things were wrong here. It ran BEFORE the index-based rebuild, so
    // deleting an Alloca shifted every later instruction down and the
    // subsequent inserts/removes addressed the wrong slots. And it deleted the
    // Alloca on the planner's say-so alone: if any reference survived (a
    // Memcpy the pass chose not to rewrite, or a Load it did not forward), the
    // value lost its definition and codegen aborted with "value N has no
    // register, stack slot, or Copy definition" (simd_sse_float).
    //
    // Now: rebuild first, then re-scan the FINAL instruction stream and delete
    // only allocas that nothing mentions any more.
    if !plan.drop_allocas.is_empty() {
        let mut referenced: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::Alloca { .. } = inst {
                    continue; // the definition itself is not a reference
                }
                let mut probe = inst.clone();
                probe.for_each_operand_mut(|op| {
                    if let Operand::Value(v) = op {
                        referenced.insert(v.0);
                    }
                });
                // Pointer fields are not Operands: collect them with the
                // exhaustive walker (Load/Store ptr, GEP base, Memcpy,
                // Intrinsic dest_ptr, va_list, ...) so a use in any of them
                // keeps the alloca alive.
                probe.for_each_value_use_mut(|v| {
                    referenced.insert(v.0);
                });
            }
            if let crate::ir::reexports::Terminator::Return(Some(Operand::Value(v))) =
                &block.terminator
            {
                referenced.insert(v.0);
            }
        }
        for block in &mut func.blocks {
            let keep: Vec<bool> = block
                .instructions
                .iter()
                .map(|inst| match inst {
                    Instruction::Alloca { dest, .. } => {
                        !(plan.drop_allocas.contains(&dest.0) && !referenced.contains(&dest.0))
                    }
                    _ => true,
                })
                .collect();
            if keep.iter().any(|&k| !k) {
                let mut i = 0;
                block.instructions.retain(|_| {
                    let k = keep[i];
                    i += 1;
                    k
                });
                if block.source_spans.len() == keep.len() {
                    let mut j = 0;
                    block.source_spans.retain(|_| {
                        let k = keep[j];
                        j += 1;
                        k
                    });
                }
            }
        }
    }

    // Re-synchronize source spans: our edits keep instruction/spans parallel
    // by construction, but any residual mismatch (inserts without spans) is
    // repaired by padding with the trailing span so downstream passes that
    // index `source_spans[inst_idx]` (mem2reg) never go out of bounds.
    for block in &mut func.blocks {
        if !block.source_spans.is_empty() && block.source_spans.len() != block.instructions.len() {
            let last = block
                .source_spans
                .last()
                .copied()
                .unwrap_or_else(crate::common::source::Span::dummy);
            block.source_spans.resize(block.instructions.len(), last);
        }
    }

    if changed > 0 {
        func.next_value_id = scan_max_value_id(func).saturating_add(1);
    }
    changed
}

#[cfg(test)]
mod intrinsic_write_contract {
    use super::*;
    use crate::ir::intrinsics::IntrinsicOp;

    #[test]
    fn pure_opcode_with_memory_result_is_a_write_barrier() {
        for dest_ptr in [None, Some(Value(0))] {
            let inst = Instruction::Intrinsic {
                dest: Some(Value(1)),
                op: IntrinsicOp::Paddw128,
                dest_ptr,
                args: vec![],
            };
            assert_eq!(is_opaque_memory_write(&inst), dest_ptr.is_some());
        }
        assert!(!is_opaque_memory_write(&Instruction::Intrinsic {
            dest: Some(Value(1)),
            op: IntrinsicOp::VecLoadI8x16,
            dest_ptr: None,
            args: vec![Operand::Value(Value(0))],
        }));
    }
}

#[cfg(test)]
mod scalar_memcpy_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, Terminator};

    #[test]
    fn lowers_small_memcpy_through_integer_typed_pointer_arithmetic() {
        let mut func = IrFunction::new("small_copy".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "src".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "dst".to_string(),
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I64(4)),
                    ty: IrType::I64,
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(2),
                    size: 4,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(lower_non_alloca_scalar_memcpy(&mut func), 1);
        let instructions = &func.blocks[0].instructions;
        assert!(matches!(
            instructions[3],
            Instruction::Load {
                ptr: Value(2),
                ty: IrType::I32,
                ..
            }
        ));
        assert!(matches!(
            instructions[4],
            Instruction::Store {
                ptr: Value(1),
                ty: IrType::I32,
                ..
            }
        ));
    }

    #[test]
    fn root_walk_is_linear_on_shared_add_dags() {
        // `a(i+1) = a(i) + a(i)` under an inttoptr cast: the root walk
        // recurses into both Add operands, so without memoization this
        // 64-deep DAG costs 2^64 node visits (measured on the equivalent
        // C source: depth 25 = 2 s, depth 30 = 52 s). With the per-value
        // memo it is one visit per value and the memcpy still lowers.
        const DEPTH: u32 = 64;
        let mut instructions = vec![Instruction::ParamRef {
            dest: Value(0),
            param_idx: 0,
            ty: IrType::I64,
        }];
        for i in 1..=DEPTH {
            instructions.push(Instruction::BinOp {
                dest: Value(i),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(i - 1)),
                rhs: Operand::Value(Value(i - 1)),
                ty: IrType::I64,
            });
        }
        instructions.push(Instruction::Cast {
            dest: Value(DEPTH + 1),
            src: Operand::Value(Value(DEPTH)),
            from_ty: IrType::I64,
            to_ty: IrType::Ptr,
        });
        instructions.push(Instruction::GlobalAddr {
            dest: Value(DEPTH + 2),
            name: "src".to_string(),
        });
        instructions.push(Instruction::Memcpy {
            dest: Value(DEPTH + 1),
            src: Value(DEPTH + 2),
            size: 4,
        });
        let mut func = IrFunction::new("dag_copy".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(lower_non_alloca_scalar_memcpy(&mut func), 1);
    }

    #[test]
    fn lowers_unaligned_eight_byte_memcpy_to_scalar_pair() {
        // memcpy at odd offsets from both roots: the scalar pair must
        // carry the full 8-byte width (backend scalar accesses are
        // misalignment-tolerant; see the contract on
        // `lower_non_alloca_scalar_memcpy`).
        let mut func = IrFunction::new("unaligned_copy".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "src".to_string(),
                },
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "dst".to_string(),
                },
                Instruction::BinOp {
                    dest: Value(2),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                },
                Instruction::BinOp {
                    dest: Value(3),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(1)),
                    rhs: Operand::Const(IrConst::I64(3)),
                    ty: IrType::I64,
                },
                Instruction::Memcpy {
                    dest: Value(3),
                    src: Value(2),
                    size: 8,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(lower_non_alloca_scalar_memcpy(&mut func), 1);
        let instructions = &func.blocks[0].instructions;
        assert!(matches!(
            instructions[4],
            Instruction::Load {
                ptr: Value(2),
                ty: IrType::I64,
                ..
            }
        ));
        assert!(matches!(
            instructions[5],
            Instruction::Store {
                ptr: Value(3),
                ty: IrType::I64,
                ..
            }
        ));
    }

    #[test]
    fn refuses_memcpy_with_opaque_pointer_root() {
        // Destination loaded from memory: its allocation root is not
        // provable by the narrow walk, so the copy stays a memcpy.
        let mut func = IrFunction::new("opaque_copy".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "slot".to_string(),
                },
                Instruction::Load {
                    dest: Value(1),
                    ptr: Value(0),
                    ty: IrType::Ptr,
                    seg_override: crate::common::types::AddressSpace::Default,
                    volatile: false,
                },
                Instruction::GlobalAddr {
                    dest: Value(2),
                    name: "src".to_string(),
                },
                Instruction::Memcpy {
                    dest: Value(1),
                    src: Value(2),
                    size: 8,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(lower_non_alloca_scalar_memcpy(&mut func), 0);
        assert!(matches!(
            func.blocks[0].instructions[3],
            Instruction::Memcpy { .. }
        ));
    }

    #[test]
    fn leaves_alloca_backed_memcpy_for_aggregate_sroa() {
        let mut func = IrFunction::new("alloca_copy".to_string(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I32,
                    size: 4,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::GlobalAddr {
                    dest: Value(1),
                    name: "src".to_string(),
                },
                Instruction::Memcpy {
                    dest: Value(0),
                    src: Value(1),
                    size: 4,
                },
            ],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        assert_eq!(lower_non_alloca_scalar_memcpy(&mut func), 0);
        assert!(matches!(
            func.blocks[0].instructions[2],
            Instruction::Memcpy { .. }
        ));
    }
}

#[cfg(test)]
mod member_copy_writer_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

    fn alloca(v: u32, size: usize) -> Instruction {
        Instruction::Alloca {
            dest: Value(v),
            ty: IrType::Ptr,
            size,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            volatile: false,
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        }
    }

    fn gep(dest: u32, base: u32, off: i64) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(off)),
            ty: IrType::Ptr,
        }
    }

    fn func(instructions: Vec<Instruction>) -> IrFunction {
        let mut f = IrFunction::new("t".into(), IrType::I64, vec![], false);
        f.next_value_id = 100;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(Some(Operand::Value(Value(6)))),
            source_spans: vec![],
        });
        f
    }

    /// `d = s; d.in = n; return d.in.x;` — the member copy into `d + 8` is a
    /// second writer of `d`, so the read must keep reading `d` (it used to be
    /// forwarded to `s + 8`, past the copy that replaced those bytes).
    #[test]
    fn member_copy_is_a_second_writer() {
        let mut f = func(vec![
            alloca(0, 24),
            alloca(1, 24),
            alloca(2, 16),
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            gep(4, 1, 8),
            Instruction::Memcpy {
                dest: Value(4),
                src: Value(2),
                size: 16,
            },
            gep(5, 1, 8),
            load(6, 5),
        ]);
        run_function(&mut f);
        let insts = &f.blocks[0].instructions;
        let Some(Instruction::Load { ptr, .. }) =
            insts.iter().find(|i| matches!(i, Instruction::Load { .. }))
        else {
            panic!("the load must survive: {insts:?}");
        };
        let mut root = ptr.0;
        while let Some(Instruction::GetElementPtr { base, .. }) = insts
            .iter()
            .find(|i| matches!(i, Instruction::GetElementPtr { dest, .. } if dest.0 == root))
        {
            root = base.0;
        }
        assert_eq!(root, 1, "d.in.x must be read from d: {insts:?}");
        assert_eq!(
            insts
                .iter()
                .filter(|i| matches!(i, Instruction::Memcpy { .. }))
                .count(),
            2,
            "both writers of d stay: {insts:?}"
        );
    }

    /// Positive control: without the member copy the read IS forwarded to
    /// `s`, so the test above fails only because of the second writer.
    #[test]
    fn single_whole_copy_still_forwards() {
        let mut f = func(vec![
            alloca(0, 24),
            alloca(1, 24),
            Instruction::Memcpy {
                dest: Value(1),
                src: Value(0),
                size: 24,
            },
            gep(5, 1, 8),
            load(6, 5),
        ]);
        assert!(run_function(&mut f) > 0);
        let insts = &f.blocks[0].instructions;
        let Some(Instruction::Load { ptr, .. }) =
            insts.iter().find(|i| matches!(i, Instruction::Load { .. }))
        else {
            panic!("the load must survive: {insts:?}");
        };
        let Some(Instruction::GetElementPtr { base, .. }) = insts
            .iter()
            .find(|i| matches!(i, Instruction::GetElementPtr { dest, .. } if dest.0 == ptr.0))
        else {
            panic!("forwarded load reads through a new GEP: {insts:?}");
        };
        assert_eq!(base.0, 0, "forwarded to s: {insts:?}");
    }
}
