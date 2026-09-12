//! Form 4 of `aggregate_sroa`: split constant-offset aggregates into fields.
//!
//! Forms 1-3 are *copy*-shaped transformations: they win when an aggregate is
//! written by one `Memcpy` and read field-wise, or the other way round. They do
//! nothing for the far more common shape in numerical kernels — a local array
//! whose elements are addressed one at a time:
//!
//! ```c
//! void core(unsigned out[16], const unsigned in[16]) {
//!     unsigned x[16];
//!     for (int i = 0; i < 16; i++) x[i] = in[i];
//!     /* 20 rounds of ARX on x[0..15], all with constant subscripts */
//!     for (int i = 0; i < 16; i++) out[i] = x[i] + in[i];
//! }
//! ```
//!
//! Every reference here is `GetElementPtr x, <constant>` + `Load`/`Store`, yet
//! the whole permutation runs against 64 bytes of stack: the quarter-round
//! macro's operands are laundered through scratch slots and stored twice, and
//! the register allocator sees one live aggregate rather than 16 independent
//! scalars. Splitting the object into 16 scalar allocas lets mem2reg give each
//! field its own SSA value, which is exactly what GCC does — and what this
//! pass must do to be competitive on ChaCha-class code.
//!
//! ## Soundness contract
//!
//! An object is split only when all six rules hold; each is checked in
//! `run_function` and the check is deliberately closed-ended (anything
//! unrecognised refuses the object rather than being ignored):
//!
//! 1. **No escape.** `any_escape` must be false for the root: no store of the
//!    address, no call argument, no `Phi`, no intrinsic operand, no
//!    `Return`. The parent's `scan()` allowlist already files every use it
//!    does not model as an escape, so a new IR instruction cannot slip through.
//! 2. **Only constant-offset interior pointers.** Any `GetElementPtr`/`Copy`
//!    chain rooted at the object with a runtime offset (rule 3's variable
//!    index) refuses the object — the address is derived from it, so we cannot
//!    know which field a later access means.
//! 3. **Every access is a scalar `Load`/`Store`.** Non-volatile, default
//!    segment, in-bounds (`off >= 0 && off + width <= size`), and each offset
//!    must be covered by exactly one scalar type: two accesses at the same
//!    offset with different types, or accesses whose widths overlap, refuse the
//!    object, because then the bytes are not independent storage.
//! 4. **The object is not claimed by forms 1-3.** No access instruction may sit
//!    at a position already in the plan (`plan.remove`, `load_ptr`,
//!    `store_ptr`, `insert`), and no `Memcpy` may still name it. Two rewrites
//!    of one instruction would be applied index-wise against the same original
//!    instruction list and the second would silently win.
//! 5. **`Memcpy` into the whole object is expanded, not refused.** A copy whose
//!    destination resolves to the object at offset 0 with `size == alloca_size`
//!    writes precisely the bytes the fields own, so it is rewritten at its own
//!    program point into one `Load`/`Store` pair per field. That keeps
//!    `memcpy(&s, &t, sizeof s)`-style initialization promotable while every
//!    other copy shape (partial copy, copy *out*, self-copy, a copy another
//!    form already planned) still refuses the object. Reading an object as one
//!    block is never expressible through independent fields, so the *source* of
//!    any `Memcpy` is always refused.
//! 6. **Size and arity caps.** At most `SPLIT_MAX_BYTES` bytes and
//!    `max_fields()` fields; a "split" into one field is a no-op and refused.
//!    Splitting widens liveness by the number of fields, and the frame
//!    allocator, not this pass, decides what fits: `CCC_AGG_SPLIT_MAX_FIELDS`
//!    bounds the fan-out (0 disables the form).
//!
//! Untouched bytes are *not* a problem: an object that cannot escape and whose
//! every read is one of the enumerated accesses has no observer for the bytes
//! between fields, so padding and never-read tails simply disappear (this is
//! the same "no unobserved bytes" rule form 3 applies to copy-outs).
//!
//! Unlike form 3, no restriction is placed on loops or back edges. Form 3 needs
//! one because it only rewrites a single basic block; this form rewires *every*
//! access of the object, so a loop-carried array becomes loop-carried phis,
//! which is what the ChaCha and hash-table shapes require.
//!
//! Enablement: **on by default**. It was held opt-in while
//! artifacts/repros/rot16-arx-O2-miscompile.md was live, because splitting a
//! hot aggregate is exactly what pushes a function onto the spill path that
//! miscomputed; that defect turned out to be an asm peephole
//! (`fold_accumulator_alu_store`) and is fixed in this series, with
//! `tests/regression/peephole_acc_fold_arx_src_kill.c` and the
//! `acc_fold_src_kill_tests` unit tests holding it down. `CCC_NO_AGGREGATE_SPLIT=1`
//! disables the form unconditionally (`CCC_AGGREGATE_SPLIT=1` remains accepted as
//! an explicit force, for A/B scripts written before the flip);
//! `CCC_AGG_SPLIT_MAX_FIELDS=n` caps fields; `CCC_AGG_SPLIT_DEBUG=1` prints the
//! per-alloca analysis and rejection reasons.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::reexports::{Instruction, IrConst, IrFunction, Operand, Value};

use super::{Insert, Plan, Scan, any_escape, const_i64, resolve, ty_size};

/// Largest object we are willing to fan out into fields.
const SPLIT_MAX_BYTES: i64 = 512;

/// Field-count cap. Generous by design — the point of the form is that a
/// 16-word state array becomes 16 registers — but a hard bound, because the
/// frame allocator's behaviour under large liveness is a separate problem
/// (see artifacts/repros/rot16-arx-O2-miscompile.md).
fn max_fields() -> usize {
    match std::env::var("CCC_AGG_SPLIT_MAX_FIELDS") {
        Ok(s) => s.trim().parse::<usize>().unwrap_or(64),
        Err(_) => 64,
    }
}

fn dbg_on() -> bool {
    std::env::var_os("CCC_AGG_SPLIT_DEBUG").is_some()
}

/// One scalar access of a candidate object.
#[derive(Clone, Copy)]
struct Access {
    block: usize,
    index: usize,
    is_load: bool,
    /// Byte offset of the access inside the object.
    off: i64,
    ty: IrType,
}

/// Everything learned about one object while walking the function.
#[derive(Default)]
struct Candidate {
    accesses: Vec<Access>,
    /// A use that cannot be re-expressed against independent fields: an
    /// escaping address, a volatile or segment-qualified access, an access
    /// whose type/offset shape is not a clean scalar, or an access a previous
    /// form already rewrote.
    refuse: bool,
    /// An interior pointer with a runtime offset exists (rule 2).
    variable: bool,
}

/// The scalar types a field may have: fixed-width integers and floats whose
/// size is a register. `Ptr` is deliberately excluded — a field holding an
/// address invites the ABI/home-slot logic that the other forms own.
fn promotable_width(ty: IrType) -> Option<i64> {
    let w = ty_size(ty);
    match w {
        1 | 2 | 4 | 8 => match ty {
            IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::F32
            | IrType::F64 => Some(w),
            _ => None,
        },
        _ => None,
    }
}

/// Walk a pointer value to the object it is derived from.
///
/// Returns `(root, offset, all_offsets_constant)`. `parent`/`koff` cover both
/// constant- and runtime-offset `GetElementPtr`s plus pointer `Copy`s, so a
/// caller can tell "derived from this object at this offset" from "derived
/// from this object, offset unknown". The 64-step bound is a trapwire against a
/// cycle produced by a malformed earlier pass; hitting it reports "not
/// constant", which only ever makes the analysis refuse.
fn chain_root(
    parent: &FxHashMap<u32, u32>,
    koff: &FxHashMap<u32, i64>,
    start: u32,
) -> (u32, i64, bool) {
    let mut v = start;
    let mut off = 0i64;
    let mut all_const = true;
    for _ in 0..64 {
        match parent.get(&v) {
            Some(&p) => {
                match koff.get(&v) {
                    Some(o) => off = off.saturating_add(*o),
                    None => all_const = false,
                }
                v = p;
            }
            None => return (v, off, all_const),
        }
    }
    (v, off, false)
}

/// Split every eligible local aggregate in `func` into per-field allocas.
///
/// Runs last among the four forms so 1-3 get first refusal on any object they
/// can express as a copy; anything they touched is skipped here. The plan is
/// filled exactly as the other forms fill it — in-place pointer rewrites,
/// indexed removals, insertions anchored on original indices — and the caller
/// applies all of it in its single index-stable rebuild per block.
pub(super) fn run_function(
    func: &IrFunction,
    scan: &Scan,
    plan: &mut Plan,
    next: &mut u32,
) -> usize {
    if std::env::var_os("CCC_NO_AGGREGATE_SPLIT").is_some() {
        return 0;
    }
    // Default on since the back-end defect this shape used to expose was fixed
    // (see the module doc). The measured corpus cost of enabling it is zero
    // instructions on every program of the golden set — the form is a
    // precondition for the ChaCha/ARX class, not a general win by itself: those
    // kernels are still refused by the copy-loop rule below until the small
    // constant-trip loops are unrolled first (P0-2).
    let max_fields = max_fields();
    if max_fields == 0 {
        return 0;
    }

    // Parameter homes are initialized by backend ABI lowering rather than by an
    // IR-visible store; splitting them would hide the ParamRef pattern from
    // mem2reg's parameter path.
    let param_homes: FxHashSet<u32> = func.param_alloca_values.iter().map(|v| v.0).collect();

    // Positions already claimed by an earlier form cannot be rewritten twice
    // (rule 4): every form indexes the ORIGINAL instruction list.
    let mut planned_pos: FxHashSet<(usize, usize)> = FxHashSet::default();
    for &(bi, ii) in &plan.remove {
        planned_pos.insert((bi, ii));
    }
    for &(bi, ii, _) in &plan.load_ptr {
        planned_pos.insert((bi, ii));
    }
    for &(bi, ii, _) in &plan.store_ptr {
        planned_pos.insert((bi, ii));
    }
    for ins in &plan.insert {
        planned_pos.insert((ins.block, ins.at));
    }

    // One pass to record how every interior pointer is derived. `parent`
    // deliberately includes GEPs with a *runtime* offset (which `Scan::gep`
    // skips): rule 2 needs those to refuse an object, and rule 3 needs them to
    // tell a constant access from a variable one.
    let mut parent: FxHashMap<u32, u32> = FxHashMap::default();
    let mut koff: FxHashMap<u32, i64> = FxHashMap::default();
    let mut allocas: Vec<u32> = Vec::new();
    let mut alloca_flags: FxHashMap<u32, (bool, bool)> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Alloca {
                    dest,
                    volatile,
                    semantic_volatile,
                    ..
                } => {
                    alloca_flags.insert(dest.0, (*volatile, *semantic_volatile));
                    allocas.push(dest.0);
                }
                Instruction::GetElementPtr {
                    dest, base, offset, ..
                } => {
                    parent.insert(dest.0, base.0);
                    if let Some(o) = const_i64(offset) {
                        koff.insert(dest.0, o);
                    }
                }
                Instruction::Copy {
                    dest,
                    src: Operand::Value(v),
                } => {
                    parent.insert(dest.0, v.0);
                    koff.insert(dest.0, 0);
                }
                _ => {}
            }
        }
    }
    let is_alloca = |v: u32| scan.alloca_size.contains_key(&v);
    // Allocas that must not be split, because a `Memcpy` names them in a shape
    // this form will not (rule 5). Populated before the access walk so the walk
    // can simply skip them.
    let mut skip: FxHashSet<u32> = FxHashSet::default();
    for &(bi, ii, d, src, size) in &scan.memcpy {
        let (dr, doff) = resolve(&scan.gep, d);
        let sr = chain_root(&parent, &koff, src).0;
        // Any whole-object read is unrepresentable through independent fields.
        skip.insert(sr);
        let expandable = doff == 0
            && scan.alloca_size.get(&dr).is_some_and(|s| *s == size)
            && sr != dr
            && !planned_pos.contains(&(bi, ii))
            // A source another form has already dropped (or that this form is
            // about to drop) would leave our inserted GEP pointing at a
            // definition that no longer exists.
            && !plan.drop_allocas.contains(&sr);
        if !expandable {
            skip.insert(dr);
        }
    }
    for &(_, _, src) in &plan.memcpy_src {
        skip.insert(src);
    }

    let mut cand: FxHashMap<u32, Candidate> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            match inst {
                Instruction::GetElementPtr { dest, .. }
                | Instruction::Copy { dest, .. }
                | Instruction::Alloca { dest, .. } => {
                    // Address derivations are handled by `chain_root`; a
                    // derivation nobody consumes is pruned at the end.
                    let _ = dest;
                }
                Instruction::Load {
                    ptr,
                    ty,
                    volatile,
                    seg_override,
                    ..
                } => {
                    let (root, off, cst) = chain_root(&parent, &koff, ptr.0);
                    if !is_alloca(root) || param_homes.contains(&root) {
                        continue;
                    }
                    let c = cand.entry(root).or_default();
                    if skip.contains(&root) {
                        c.refuse = true;
                        continue;
                    }
                    if *volatile
                        || !matches!(seg_override, AddressSpace::Default)
                        || !cst
                        || planned_pos.contains(&(bi, ii))
                    {
                        if !cst {
                            c.variable = true;
                            if dbg_on() {
                                eprintln!(
                                    "[split] VARIABLE load root v{} at block {} inst {}",
                                    root, bi, ii
                                );
                            }
                        }
                        c.refuse = true;
                        continue;
                    }
                    if promotable_width(*ty).is_none() || off < 0 {
                        c.refuse = true;
                        continue;
                    }
                    c.accesses.push(Access {
                        block: bi,
                        index: ii,
                        is_load: true,
                        off,
                        ty: *ty,
                    });
                }
                Instruction::Store {
                    ptr,
                    ty,
                    volatile,
                    seg_override,
                    ..
                } => {
                    let (root, off, cst) = chain_root(&parent, &koff, ptr.0);
                    if !is_alloca(root) || param_homes.contains(&root) {
                        continue;
                    }
                    let c = cand.entry(root).or_default();
                    if skip.contains(&root) {
                        c.refuse = true;
                        continue;
                    }
                    if *volatile
                        || !matches!(seg_override, AddressSpace::Default)
                        || !cst
                        || planned_pos.contains(&(bi, ii))
                    {
                        if !cst {
                            c.variable = true;
                            if dbg_on() {
                                eprintln!(
                                    "[split] VARIABLE store root v{} at block {} inst {}",
                                    root, bi, ii
                                );
                            }
                        }
                        c.refuse = true;
                        continue;
                    }
                    if promotable_width(*ty).is_none() || off < 0 {
                        c.refuse = true;
                        continue;
                    }
                    c.accesses.push(Access {
                        block: bi,
                        index: ii,
                        is_load: false,
                        off,
                        ty: *ty,
                    });
                }
                // A `Memcpy` naming the object was already accounted for above
                // (either recorded for expansion or pushed into `skip`).
                Instruction::Memcpy { .. } => {}
                // Everything else that reads or writes a value: refuse the
                // object it touches. This mirrors (and has to repeat) the
                // parent's allowlist sweep, because that one records *operands*
                // only — and several memory destinations are raw `Value` fields
                // instead: `VaArgStruct { dest_ptr, va_list_ptr }`, `VaArg`,
                // `AtomicStore`, `Intrinsic { dest_ptr }`. Missing one of those
                // is not a missed optimization: the split drops the alloca the
                // instruction writes into and codegen aborts with
                // "value N has no register, stack slot, Copy, or GlobalAddr
                // definition" (found by tests/regression/va_arg_wide_struct.c
                // with this form enabled). `used_values()` walks every value
                // field, so nothing has to be enumerated here.
                other => {
                    for v in other.used_values() {
                        let (root, _, _) = chain_root(&parent, &koff, v);
                        if is_alloca(root) && !param_homes.contains(&root) {
                            cand.entry(root).or_default().refuse = true;
                        }
                    }
                }
            }
        }
        // Control flow naming a pointer is an escape no field decomposition can
        // express: `return &x[0]`, or a branch on an interior pointer.
        for v in block.terminator.used_values() {
            let (root, _, _) = chain_root(&parent, &koff, v);
            if is_alloca(root) && !param_homes.contains(&root) {
                cand.entry(root).or_default().refuse = true;
            }
        }

        // Cross-check the refusal set against the canonical memory-write
        // predicate (`Instruction::may_write_memory`, exhaustive over opcodes,
        // upstream 43cb636b). This module's sweep is deliberately *broader*: it
        // also refuses on unmodelled reads, which that predicate answers `false`
        // for. The invariant is therefore one-directional -- every instruction
        // that may write memory and names a candidate root must have marked that
        // root refused -- and the two forms this pass rewrites in place are
        // exempt because their writes are what the split models: `Store` becomes
        // a field store, and a whole-object `Memcpy` is expanded into per-field
        // copies at its own program point.
        //
        // What it guards: the arms above that *accept* an instruction
        // (`Alloca`, `GetElementPtr`, `Load`, `Phi`-ish copies) assume those
        // opcodes touch no memory but their root. If an opcode ever gains a
        // memory destination, a per-opcode `match` keeps accepting it and the
        // split drops the alloca underneath the write -- the same failure the
        // `VaArgStruct` blind spot produced ("value N has no register, stack
        // slot, Copy, or GlobalAddr definition"). A new writing opcode landing in
        // the fallback arm is already refused; this catches drift in the *handled*
        // set, and keeps the two predicates from silently disagreeing as either
        // one evolves.
        let violation = if cfg!(debug_assertions) || dbg_on() {
            block.instructions.iter().find(|inst| {
                if !inst.may_write_memory()
                    || matches!(inst, Instruction::Store { .. } | Instruction::Memcpy { .. })
                {
                    return false;
                }
                inst.used_values().iter().any(|v| {
                    let (root, _, _) = chain_root(&parent, &koff, *v);
                    matches!(cand.get(&root), Some(c) if !c.refuse)
                })
            })
        } else {
            None
        };
        if let Some(inst) = violation {
            // A bare `debug_assert!` would be dead code in the profile the
            // regression suite and the corpus gates actually run: `fastbuild`
            // inherits `release`, so debug_assertions are off -- which is exactly
            // how a guard like this comes to read as "checked" while never
            // firing. The same condition therefore also reports under
            // `CCC_AGG_SPLIT_DEBUG=1`, and `tests/regression/
            // aggregate_split_constant_fields.c.env` sets that so the shipped
            // suite exercises the invariant on a split-heavy file. Terminators are
            // not re-examined: the sweep above refuses every root a terminator
            // names, unconditionally.
            let msg = format!(
                "[SROA-split] {}: block {bi} has a memory-writing instruction that names a \
                 candidate this pass did not refuse -- this module's model and \
                 `may_write_memory` disagree: {inst:?}",
                func.name,
            );
            if cfg!(debug_assertions) {
                panic!("{msg}");
            }
            eprintln!("{msg}");
        }
    }

    // Now that every candidate exists, refuse the ones whose address escaped
    // (rule 1) and the ones a previous form dropped or is about to drop.
    for (root, c) in cand.iter_mut() {
        if any_escape(&scan.escapes, &scan.gep, *root) {
            c.refuse = true;
        }
        if plan.drop_allocas.contains(root) {
            c.refuse = true;
        }
        if let Some(&(v, sv)) = alloca_flags.get(root) {
            if v || sv {
                c.refuse = true;
            }
        }
    }

    let mut split_roots: Vec<u32> = Vec::new();
    let mut rewrote: FxHashSet<(usize, usize)> = FxHashSet::default();
    let mut total = 0usize;

    for root in allocas {
        let Some(c) = cand.get(&root) else { continue };
        let size = scan.alloca_size.get(&root).copied().unwrap_or(0);
        if dbg_on() {
            eprintln!(
                "[split] root v{} size={} accesses={} refuse={} variable={} escapes={} memcpy_skip={}",
                root,
                size,
                c.accesses.len(),
                c.refuse,
                c.variable,
                any_escape(&scan.escapes, &scan.gep, root),
                skip.contains(&root),
            );
        }
        if c.refuse
            || c.variable
            || skip.contains(&root)
            || c.accesses.len() < 2
            || size <= 0
            || size > SPLIT_MAX_BYTES
        {
            continue;
        }

        // Group accesses by offset and prove the fields are independent
        // (rule 3).
        let mut fields: Vec<(i64, IrType, i64)> = Vec::new();
        let mut malformed = false;
        for a in &c.accesses {
            let w = match promotable_width(a.ty) {
                Some(w) => w,
                None => {
                    malformed = true;
                    break;
                }
            };
            if a.off + w > size {
                malformed = true;
                break;
            }
            match fields.iter().position(|&(o, _, _)| o == a.off) {
                Some(k) => {
                    if fields[k].1 != a.ty {
                        malformed = true;
                        break;
                    }
                }
                None => fields.push((a.off, a.ty, w)),
            }
        }
        if malformed {
            if dbg_on() {
                eprintln!("[split]   v{} rejected: overlapping or unsized views", root);
            }
            continue;
        }
        if fields.len() < 2 || fields.len() > max_fields {
            continue;
        }
        fields.sort_by_key(|&(o, _, _)| o);
        // Equal offsets were merged above, so comparing neighbours rules out
        // every overlap.
        if fields.windows(2).any(|w2| w2[0].0 + w2[0].2 > w2[1].0) {
            if dbg_on() {
                eprintln!("[split]   v{} rejected: fields overlap", root);
            }
            continue;
        }

        let mut field_id: FxHashMap<i64, u32> = FxHashMap::default();
        for &(off, ty, w) in &fields {
            let id = *next;
            *next += 1;
            field_id.insert(off, id);
            plan.new_allocas.push(Instruction::Alloca {
                dest: Value(id),
                ty,
                size: w as usize,
                // 0 asks the frame layouter for the type's natural alignment.
                align: 0,
                volatile: false,
                semantic_volatile: false,
            });
        }

        for a in &c.accesses {
            let id = match field_id.get(&a.off) {
                Some(&id) => id,
                None => continue,
            };
            if a.is_load {
                plan.load_ptr.push((a.block, a.index, id));
            } else {
                plan.store_ptr.push((a.block, a.index, id));
            }
            rewrote.insert((a.block, a.index));
        }

        // Rule 5: expand each whole-object copy-in at its own program point.
        // The sequence reads each source byte exactly once and writes each
        // destination byte exactly once, so it is equivalent to the block copy
        // for a size-matching scalar type; the object's address does not escape
        // (rule 1), so no observer can see the difference.
        // Every inserted instruction shares the memcpy's own anchor: the
        // rebuild's insert sort is stable, so planner order (GEP, Load, Store)
        // is what reaches the block, and all of them land in the slot the
        // removed copy occupied. Anchoring them at `ii + k` instead interleaves
        // them with the *following* original instructions, so a later field's
        // store lands after the uses that read it (observed: a 4-field struct
        // copy-in computed 86 instead of 152).
        for &(bi, ii, d, src, _size) in &scan.memcpy {
            let (dr, doff) = resolve(&scan.gep, d);
            if dr != root || doff != 0 {
                continue;
            }
            for &(off, ty, _w) in &fields {
                let field = match field_id.get(&off) {
                    Some(&f) => f,
                    None => continue,
                };
                let gid = *next;
                *next += 1;
                plan.insert.push(Insert {
                    block: bi,
                    at: ii,
                    inst: Instruction::GetElementPtr {
                        dest: Value(gid),
                        base: Value(src),
                        offset: Operand::Const(IrConst::ptr_int(off)),
                        ty: IrType::Ptr,
                    },
                });
                let lid = *next;
                *next += 1;
                plan.insert.push(Insert {
                    block: bi,
                    at: ii,
                    inst: Instruction::Load {
                        volatile: false,
                        dest: Value(lid),
                        ptr: Value(gid),
                        ty,
                        seg_override: AddressSpace::Default,
                    },
                });
                plan.insert.push(Insert {
                    block: bi,
                    at: ii,
                    inst: Instruction::Store {
                        volatile: false,
                        val: Operand::Value(Value(lid)),
                        ptr: Value(field),
                        ty,
                        seg_override: AddressSpace::Default,
                    },
                });
            }
            plan.remove.push((bi, ii));
            rewrote.insert((bi, ii));
        }

        plan.drop_allocas.insert(root);
        split_roots.push(root);
        total += 1;
        if dbg_on() || std::env::var_os("CCC_DEBUG_SROA").is_some() {
            eprintln!(
                "[SROA-split] fn {} alloca v{} size={} fields={}",
                func.name,
                root,
                size,
                fields.len()
            );
        }
    }

    if total > 0 {
        prune_dead_interior_pointers(func, &parent, &split_roots, &rewrote, plan);
    }
    total
}

/// Remove the address derivations that the split left without consumers.
///
/// Every `x[i]` was a `GetElementPtr` rooted at the object; after the accesses
/// are redirected to field allocas those GEPs are dead, but the parent's
/// drop-check counts a `GetElementPtr { base }` as a reference — so unless they
/// go, the aggregate alloca stays in the frame and the split buys nothing but
/// instruction churn (measured on ChaCha: 633 -> 603 instead of a real win).
///
/// A derivation is removed only when *every* one of its users is an instruction
/// this form rewrote or removed, transitively: chains are followed by a
/// fixpoint, and a value used by a terminator or by anything else is kept.
fn prune_dead_interior_pointers(
    func: &IrFunction,
    parent: &FxHashMap<u32, u32>,
    roots: &[u32],
    rewrote: &FxHashSet<(usize, usize)>,
    plan: &mut Plan,
) {
    let want: FxHashSet<u32> = roots.iter().copied().collect();

    // value -> users, plus the values a terminator names (those are never
    // removable here: the parent's re-scan counts them and control flow does).
    let mut users: FxHashMap<u32, Vec<(usize, usize)>> = FxHashMap::default();
    let mut in_terminator: FxHashSet<u32> = FxHashSet::default();
    let mut positions: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            if let Some(d) = inst.dest() {
                positions.insert(d.0, (bi, ii));
            }
            let mut probe = inst.clone();
            probe.for_each_operand_mut(|op| {
                if let Operand::Value(v) = op {
                    users.entry(v.0).or_default().push((bi, ii));
                }
            });
            // Bare-Value positions (Load/Store ptr, GEP base, Memcpy,
            // va_list, ...) are users too; the exhaustive walker covers
            // them all (a hand-rolled match missed positions and understated
            // the user map).
            probe.for_each_value_use_mut(|v| {
                users.entry(v.0).or_default().push((bi, ii));
            });
        }
        for v in block.terminator.used_values() {
            in_terminator.insert(v);
        }
    }

    // Candidate derivations: every pointer derived from a split root.
    let mut derived: FxHashMap<u32, bool> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            let (dest, base) = match inst {
                Instruction::GetElementPtr { dest, base, .. } => (dest.0, base.0),
                Instruction::Copy {
                    dest,
                    src: Operand::Value(v),
                } => (dest.0, v.0),
                _ => continue,
            };
            if parent.contains_key(&dest) && want.contains(&root_of(parent, base)) {
                derived.insert(dest, false);
            }
        }
    }

    // Fixpoint: a derivation dies when all of its users are dead or rewritten.
    let mut changed = true;
    while changed {
        changed = false;
        // Decisions are collected in a scratch list: `derived` is read from
        // inside the user test, so it cannot also be iterated mutably.
        let live: Vec<u32> = derived
            .iter()
            .filter(|(_, dead)| !**dead)
            .map(|(v, _)| *v)
            .collect();
        for v in live {
            if in_terminator.contains(&v) {
                continue;
            }
            let all_dead = match users.get(&v) {
                // Nothing reads this derivation at all.
                None => true,
                Some(list) => list.iter().all(|&(bi, ii)| {
                    // The user may stay if it is an access we rewired, or a
                    // derivation already marked dead in an earlier round.
                    rewrote.contains(&(bi, ii))
                        || match func.blocks[bi].instructions.get(ii).and_then(|i| i.dest()) {
                            Some(d) => derived.get(&d.0) == Some(&true),
                            None => false,
                        }
                }),
            };
            if all_dead {
                derived.insert(v, true);
                changed = true;
            }
        }
    }

    let mut removed: FxHashSet<(usize, usize)> = plan.remove.iter().copied().collect();
    for (v, dead) in &derived {
        if !*dead {
            continue;
        }
        let Some(&(bi, ii)) = positions.get(v) else {
            continue;
        };
        if removed.insert((bi, ii)) {
            plan.remove.push((bi, ii));
        }
    }
}

/// Root of `v` following only the derivation edges (offsets irrelevant).
fn root_of(parent: &FxHashMap<u32, u32>, mut v: u32) -> u32 {
    for _ in 0..64 {
        match parent.get(&v) {
            Some(&p) => v = p,
            None => return v,
        }
    }
    v
}
