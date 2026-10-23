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
//!    **3b. Covering accesses** are the one admitted overlap: an *integer*
//!    access that wholly contains *integer* fields (`I64 @0` over two `U32`
//!    members -- how every 9..=16-byte struct crosses a call in two
//!    registers, and therefore what an inlined struct return or by-value
//!    argument leaves behind) is rebuilt from the fields at its own program
//!    point: a load as `or(zext(f_i) << 8*rel_i)`, a store as one
//!    `trunc(v >> 8*rel_i)` per field (all LCCC targets are little-endian).
//!    Gaps inside a covering access get carrier fields (see rule 5), so every
//!    byte it moves has an owner. A covering access spans at most
//!    `MAX_COVERING_PARTS` (4) parts: past that the rebuild costs more than
//!    the memory round trip it removes. `field_layout` holds the exact
//!    conditions.
//! 4. **The object is not claimed by forms 1-3.** No access instruction may sit
//!    at a position already in the plan (`plan.remove`, `load_ptr`,
//!    `store_ptr`, `insert`), and no `Memcpy` may still name it. Two rewrites
//!    of one instruction would be applied index-wise against the same original
//!    instruction list and the second would silently win.
//! 5. **Whole-object `Memcpy` in either direction is expanded, not refused.**
//!    *Copy-in:* a copy whose destination resolves to the object at offset 0
//!    with `size == alloca_size` writes precisely the bytes the fields own, so
//!    it is rewritten at its own program point into one `Load`/`Store` pair
//!    per field. That keeps `memcpy(&s, &t, sizeof s)`-style initialization
//!    promotable.
//!    *Copy-out:* a copy whose source is the whole object (offset 0, full
//!    size) into memory that is neither derived from the object nor rooted at
//!    any local alloca becomes one field load plus one destination store per
//!    field, again at the copy's program point. The object cannot escape, so
//!    the destination cannot alias it and the reads cannot observe the
//!    writes. Because every byte of the object is now observable at the
//!    destination, the fields must tile it: each gap between fields (padding,
//!    never-touched bytes, an uneven tail) gets a *carrier* field, the widest
//!    naturally aligned integer that fits. A copy-in fills carriers like any
//!    other field; without one they carry the same indeterminate bytes the
//!    block copy would have moved. This is the `fast_args` shape of
//!    `HUF_decompress4X*_usingDTable_internal_fast_c_loop` in zstd: copy the
//!    argument block in, run the loop on the fields, copy it back out.
//!    Every other shape still refuses the object: a partial copy, a self-copy,
//!    a copy between two locals on the source side (the copy-in rule may
//!    still split the destination), a copy another form already planned, or
//!    a copy whose other end is claimed by an object this form splits or
//!    drops in the same run (both rewrites would name the same instruction).
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
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, IrFunction, Operand, Value};

use super::{Insert, Plan, Scan, any_escape, const_i64, resolve, ty_size};

/// Largest object we are willing to fan out into fields.
const SPLIT_MAX_BYTES: i64 = 512;

/// The form's configuration, resolved ONCE per `run_passes` from the process
/// environment and then read from thread-local state.
///
/// WHY NOT `std::env::var` HERE. This form runs per function, so reading the
/// environment in it was an `environ` scan (plus a `String` allocation for the
/// numeric knob) for every function in the translation unit -- and, because the
/// environment is process-global, each kill-switch test had to mutate it
/// underneath every sibling test on cargo's thread pool. The house rule is in
/// `src/passes/mod.rs` ("Every remaining pass switch, resolved here for the
/// same reason"): the pipeline resolves the switches once and the pass reads
/// per-thread state, which is also what keeps `check_env_test_hygiene.sh`'s
/// ratchet meaningful. Unit tests that never go through `run_passes` see
/// `SPLIT_ENV_DEFAULT`, i.e. the all-unset configuration.
#[derive(Clone, Copy)]
pub(crate) struct SplitEnv {
    /// Whether the form runs at all: `CCC_NO_AGGREGATE_SPLIT` unset.
    pub(crate) enabled: bool,
    /// Whether refusals and decisions are explained: `CCC_AGG_SPLIT_DEBUG`.
    pub(crate) debug: bool,
    /// Field-count cap. Generous by design -- the point of the form is that a
    /// 16-word state array becomes 16 registers -- but a hard bound, because
    /// the frame allocator's behaviour under large liveness is a separate
    /// problem (see artifacts/repros/rot16-arx-O2-miscompile.md). Zero means
    /// "refuse everything", matching the historical `CCC_AGG_SPLIT_MAX_FIELDS=0`
    /// spelling.
    pub(crate) max_fields: usize,
    /// `CCC_DEBUG_SROA`: the whole of `aggregate_sroa` traces to stderr --
    /// the per-function change count (`[SROA]`, `mod.rs`) and every split
    /// (`[SROA-split]`, here). Resolved with the form's own switches so both
    /// trace sites agree on its spelling and neither scans `environ` per
    /// function.
    pub(crate) sroa_trace: bool,
}

/// The environment-unset configuration, which is also what an entry point that
/// never went through `run_passes` (unit tests) observes.
pub(crate) const SPLIT_ENV_DEFAULT_MAX_FIELDS: usize = 64;
const SPLIT_ENV_DEFAULT: SplitEnv = SplitEnv {
    enabled: true,
    debug: false,
    max_fields: SPLIT_ENV_DEFAULT_MAX_FIELDS,
    sroa_trace: false,
};

thread_local! {
    static SPLIT_ENV: std::cell::Cell<SplitEnv> = const { std::cell::Cell::new(SPLIT_ENV_DEFAULT) };
}

pub(crate) fn set_split_env(env: SplitEnv) {
    SPLIT_ENV.with(|cell| cell.set(env));
}

#[inline]
fn split_env() -> SplitEnv {
    SPLIT_ENV.with(std::cell::Cell::get)
}

/// [`SplitEnv::sroa_trace`] for the rest of `aggregate_sroa`.
#[inline]
pub(super) fn sroa_trace() -> bool {
    split_env().sroa_trace
}

#[inline]
fn dbg_on() -> bool {
    split_env().debug
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
/// size is a register, plus `Ptr`. A pointer field is itself one
/// register-sized value, so it promotes like any other scalar; the
/// address-escape refusal in `run_function` (an alloca whose address is
/// stored as a value) is what keeps the ABI/home-slot hazards out.
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
            | IrType::F64
            | IrType::Ptr => Some(w),
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
/// The value a `memset(.., byte, ..)` leaves in a field of type `ty`: `byte`
/// repeated over the field's width. Integers and pointers take any byte
/// (`from_i64` keeps unsigned types zero-extended and pointers at the
/// target's width). A float field is admitted only for byte 0, whose all-zero
/// bit pattern is `+0.0`: any other byte yields an arbitrary pattern, possibly
/// a signalling NaN, whose bits constant folding need not preserve.
fn memset_splat(ty: IrType, byte: u8) -> Option<IrConst> {
    let w = promotable_width(ty)?;
    let splat = (u64::from(byte)).wrapping_mul(0x0101_0101_0101_0101);
    let bits = if w == 8 {
        splat
    } else {
        splat & ((1u64 << (8 * w)) - 1)
    };
    match ty {
        IrType::F32 | IrType::F64 => (byte == 0).then(|| IrConst::from_i64(0, ty)),
        _ => Some(IrConst::from_i64(bits as i64, ty)),
    }
}

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
    if !split_env().enabled {
        return 0;
    }
    // Default on since the back-end defect this shape used to expose was fixed
    // (see the module doc). The measured corpus cost of enabling it is zero
    // instructions on every program of the golden set — the form is a
    // precondition for the ChaCha/ARX class, not a general win by itself: those
    // kernels are still refused by the copy-loop rule below until the small
    // constant-trip loops are unrolled first (P0-2).
    let max_fields = split_env().max_fields;
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
        // A whole-object read is expressible through the fields only as a
        // copy-OUT into memory that is not itself a local object: one field
        // load and one store per field (gaps carried, see rule 5). A copy
        // between two locals stays refused on the source side; the copy-in
        // rule below may still split the destination.
        let dest_root = chain_root(&parent, &koff, d).0;
        let copy_out = resolve(&scan.gep, src).1 == Some(0)
            && scan.alloca_size.get(&sr).is_some_and(|s| *s == size)
            && dest_root != sr
            && !is_alloca(dest_root)
            && !planned_pos.contains(&(bi, ii));
        if !copy_out {
            skip.insert(sr);
        }
        let expandable = doff == Some(0)
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
    // Rule 5b: a whole-object memset is expanded, not refused. A constant
    // library memset (`super::const_memset`) whose pointer resolves to a
    // local object at offset 0 with `len == alloca size`, whose result is
    // unused and which no earlier form claimed writes precisely the bytes the
    // fields own; it is rewritten at its own program point into one constant
    // store per field. Every other memset naming a local object (partial,
    // offset, result used, claimed) refuses that object, exactly as the
    // parent's escape set would have.
    let mut memset_of: FxHashMap<u32, Vec<(usize, usize, u8)>> = FxHashMap::default();
    let mut memset_pos: FxHashSet<(usize, usize)> = FxHashSet::default();
    if !scan.memset.is_empty() {
        let mut used: FxHashSet<u32> = FxHashSet::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                inst.for_each_used_value(|v| {
                    used.insert(v);
                });
            }
            block.terminator.for_each_used_value(|v| {
                used.insert(v);
            });
        }
        for &(bi, ii, p, byte, len, result) in &scan.memset {
            let (root, off, cst) = chain_root(&parent, &koff, p);
            if !is_alloca(root) {
                continue;
            }
            let whole = cst
                && off == 0
                && scan.alloca_size.get(&root) == Some(&len)
                && result.is_none_or(|r| !used.contains(&r.0))
                && !planned_pos.contains(&(bi, ii));
            if whole {
                memset_of.entry(root).or_default().push((bi, ii, byte));
                memset_pos.insert((bi, ii));
            } else {
                skip.insert(root);
            }
        }
    }

    let mut cand: FxHashMap<u32, Candidate> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            // An accepted whole-object memset (rule 5b) is rewritten with its
            // object; its only value operand is that object's address.
            if memset_pos.contains(&(bi, ii)) {
                continue;
            }
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
                    val,
                    ptr,
                    ty,
                    volatile,
                    seg_override,
                } => {
                    if let Operand::Value(v) = val {
                        let (val_root, _, _) = chain_root(&parent, &koff, v.0);
                        if is_alloca(val_root) && !param_homes.contains(&val_root) {
                            cand.entry(val_root).or_default().refuse = true;
                        }
                    }
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
            block
                .instructions
                .iter()
                .enumerate()
                .find_map(|(ii, inst)| {
                    if !inst.may_write_memory()
                        || matches!(inst, Instruction::Store { .. } | Instruction::Memcpy { .. })
                        || memset_pos.contains(&(bi, ii))
                    {
                        return None;
                    }
                    inst.used_values()
                        .iter()
                        .any(|v| {
                            let (root, _, _) = chain_root(&parent, &koff, *v);
                            matches!(cand.get(&root), Some(c) if !c.refuse)
                        })
                        .then_some(inst)
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
        if any_escape(scan.split_escapes(), &scan.gep, *root) {
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
    // Roots a committed copy-out writes into through inserted GEPs.
    let mut copy_out_targets: FxHashSet<u32> = FxHashSet::default();
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
                any_escape(scan.split_escapes(), &scan.gep, root),
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

        // Group accesses into views and prove the fields are independent
        // (rule 3, with rule 3b's covering accesses).
        let Some((mut fields, covering)) = field_layout(&c.accesses, size) else {
            if dbg_on() {
                eprintln!("[split]   v{} rejected: overlapping or unsized views", root);
            }
            continue;
        };
        // The whole-object copies this object takes part in, and the guard
        // against rewriting one instruction twice: a copy between two objects
        // this run splits is claimed by whichever is split first, and the other
        // then keeps its memory. A copy-out's inserted address arithmetic names
        // its destination, so an object some copy-out already writes into
        // cannot be dropped either.
        let copies_in: Vec<(usize, usize, u32)> = scan
            .memcpy
            .iter()
            .filter(|&&(_, _, d, _, _)| resolve(&scan.gep, d) == (root, Some(0)))
            .map(|&(bi, ii, _, src, _)| (bi, ii, src))
            .collect();
        let copies_out: Vec<(usize, usize, u32)> = scan
            .memcpy
            .iter()
            .filter(|&&(_, _, d, src, _)| {
                chain_root(&parent, &koff, src).0 == root && chain_root(&parent, &koff, d).0 != root
            })
            .map(|&(bi, ii, d, _, _)| (bi, ii, d))
            .collect();
        if copies_in
            .iter()
            .chain(&copies_out)
            .any(|&(bi, ii, _)| rewrote.contains(&(bi, ii)))
            || copy_out_targets.contains(&root)
            || copies_out.iter().any(|&(_, _, d)| {
                let dr = chain_root(&parent, &koff, d).0;
                plan.drop_allocas.contains(&dr) || split_roots.contains(&dr)
            })
        {
            if dbg_on() {
                eprintln!(
                    "[split]   v{} rejected: copy claimed by another object",
                    root
                );
            }
            continue;
        }
        // Gap carriers (rule 5): a copy-out makes every byte observable at the
        // destination, so the fields must tile the object. Each gap is covered
        // by the widest naturally aligned integer that fits; a copy-in fills
        // the carriers like any other field, and with no writer they read as
        // the indeterminate bytes the block copy would have moved.
        if !copies_out.is_empty() {
            let mut carriers: Vec<(i64, IrType, i64)> = Vec::new();
            let mut pos = 0i64;
            for &(off, _, w) in fields.iter().chain(std::iter::once(&(size, IrType::I8, 0))) {
                while pos < off {
                    let (ty, w) = [(IrType::I64, 8i64), (IrType::I32, 4), (IrType::I16, 2)]
                        .into_iter()
                        .find(|&(_, w)| pos % w == 0 && pos + w <= off)
                        .unwrap_or((IrType::I8, 1));
                    carriers.push((pos, ty, w));
                    pos += w;
                }
                pos = pos.max(off + w);
            }
            fields.extend(carriers);
            fields.sort_by_key(|&(o, _, _)| o);
        }
        if fields.len() < 2 || fields.len() > max_fields {
            continue;
        }
        // Rule 5b: every field (carriers included) needs the exact constant a
        // whole-object memset leaves in it; refuse the object otherwise.
        let memsets: &[(usize, usize, u8)] = memset_of.get(&root).map_or(&[], Vec::as_slice);
        if memsets.iter().any(|&(bi, ii, byte)| {
            rewrote.contains(&(bi, ii))
                || fields
                    .iter()
                    .any(|&(_, ty, _)| memset_splat(ty, byte).is_none())
        }) {
            if dbg_on() {
                eprintln!(
                    "[split]   v{} rejected: memset not expressible per field",
                    root
                );
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

        let field_ty: FxHashMap<i64, IrType> =
            fields.iter().map(|&(off, ty, _)| (off, ty)).collect();
        for a in &c.accesses {
            if covering.contains(&(a.off, ty_size(a.ty))) {
                // Rule 3b: rebuild the covering access from its fields at its
                // own program point (stable anchor, as for copy-in below).
                let parts: Vec<(i64, IrType, u32)> = fields
                    .iter()
                    .filter(|&&(off, _, _)| off >= a.off && off < a.off + ty_size(a.ty))
                    .filter_map(|&(off, ty, _)| field_id.get(&off).map(|&id| (off - a.off, ty, id)))
                    .collect();
                let inst = &func.blocks[a.block].instructions[a.index];
                let seq = match inst {
                    Instruction::Load { dest, .. } => covering_load(*dest, a.ty, &parts, next),
                    Instruction::Store { val, .. } => covering_store(*val, a.ty, &parts, next),
                    _ => unreachable!("access records only Load/Store"),
                };
                for inst in seq {
                    plan.insert.push(Insert {
                        block: a.block,
                        at: a.index,
                        inst,
                    });
                }
                plan.remove.push((a.block, a.index));
                rewrote.insert((a.block, a.index));
                continue;
            }
            let id = match field_id.get(&a.off) {
                Some(&id) => id,
                None => continue,
            };
            let fty = field_ty[&a.off];
            if fty != a.ty {
                // A sign variant of the field's type (`field_layout` merged
                // it): access the field at its own type, cast at the point.
                let inst = &func.blocks[a.block].instructions[a.index];
                for inst in retyped_access(inst, fty, id, next) {
                    plan.insert.push(Insert {
                        block: a.block,
                        at: a.index,
                        inst,
                    });
                }
                plan.remove.push((a.block, a.index));
                rewrote.insert((a.block, a.index));
                continue;
            }
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
        for &(bi, ii, src) in &copies_in {
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
        // Rule 5, copy-out: the mirror image, one field load and one store
        // into the destination per field at the copy's own program point. The
        // fields tile the object (carriers above), so every destination byte
        // the block copy wrote is written, each exactly once; the destination
        // is not derived from the object, so the reads cannot see the writes.
        for &(bi, ii, d) in &copies_out {
            copy_out_targets.insert(chain_root(&parent, &koff, d).0);
            for &(off, ty, _w) in &fields {
                let Some(&field) = field_id.get(&off) else {
                    continue;
                };
                let lid = *next;
                *next += 1;
                plan.insert.push(Insert {
                    block: bi,
                    at: ii,
                    inst: Instruction::Load {
                        volatile: false,
                        dest: Value(lid),
                        ptr: Value(field),
                        ty,
                        seg_override: AddressSpace::Default,
                    },
                });
                let gid = *next;
                *next += 1;
                plan.insert.push(Insert {
                    block: bi,
                    at: ii,
                    inst: Instruction::GetElementPtr {
                        dest: Value(gid),
                        base: Value(d),
                        offset: Operand::Const(IrConst::ptr_int(off)),
                        ty: IrType::Ptr,
                    },
                });
                plan.insert.push(Insert {
                    block: bi,
                    at: ii,
                    inst: Instruction::Store {
                        volatile: false,
                        val: Operand::Value(Value(lid)),
                        ptr: Value(gid),
                        ty,
                        seg_override: AddressSpace::Default,
                    },
                });
            }
            plan.remove.push((bi, ii));
            rewrote.insert((bi, ii));
        }

        // Rule 5b: each whole-object memset becomes one constant store per
        // field at its own program point (same stable-anchor rule as the
        // copy-in above). The bytes between fields have no observer unless a
        // copy-out exists, and then carriers tile them and are stored too.
        for &(bi, ii, byte) in memsets {
            for &(off, ty, _w) in &fields {
                let (Some(&field), Some(val)) = (field_id.get(&off), memset_splat(ty, byte)) else {
                    continue;
                };
                plan.insert.push(Insert {
                    block: bi,
                    at: ii,
                    inst: Instruction::Store {
                        volatile: false,
                        val: Operand::Const(val),
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
        if dbg_on() || sroa_trace() {
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

/// The field layout of one candidate (rules 3 and 3b): the fields as
/// `(offset, type, width)` sorted by offset, plus the covering views
/// `(offset, type)` that are rebuilt from fields instead of being one.
///
/// Rule 3: every byte belongs to at most one field, and all accesses to a
/// field agree on its type. Rule 3b relaxes "accesses must not overlap" for
/// exactly one shape: an INTEGER access that covers whole INTEGER fields.
/// ABI carriers produce it everywhere -- a 12-byte `{u32 a, b, c}` returned
/// or passed in two registers is read as `I64 @0` + `U32 @8` and written
/// back the same way, so without 3b no such struct ever split once its
/// callee was inlined (zstd's `repcodes_t`). A covering load becomes
/// `or(zext(f_i) << 8*(off_i - off))` over its fields, a covering store
/// becomes one `trunc(v >> 8*(off_i - off))` per field: the byte layout of
/// every LCCC target (x86-64, i686, AArch64, RISC-V64) is little-endian.
///
/// The fields are the MINIMAL views (those containing no other view);
/// a view that contains another is covering and must:
///   * be an integer type (`Ptr` and floats need a bit cast, not a shift);
///   * contain only integer fields, each entirely (a field straddling the
///     covering view's edge means the bytes are not independent storage);
///   * and have every byte owned: a gap inside it (padding between two
///     members) gets a carrier field, the widest naturally aligned integer
///     that fits, which reads back the bytes the wide access last wrote.
/// Two INTEGER views of the same bytes that differ only in signedness
/// (`I32` zero-fill store, `U32` carrier load) are one view: the field takes
/// the type of the first access in program order and the run rewrites the
/// others with a same-width cast (mem2reg promotes only exact-type
/// accesses). Any other pair of views on the same bytes with different types
/// (`I32`/`F32`, `I64`/`Ptr`) still refuses the object, as before.
///
/// A covering view may span at most [`MAX_COVERING_PARTS`] parts (fields
/// plus carriers); see there for the cost model.
fn field_layout(
    accesses: &[Access],
    size: i64,
) -> Option<(Vec<(i64, IrType, i64)>, FxHashSet<(i64, i64)>)> {
    let mut views: Vec<(i64, IrType, i64)> = Vec::new();
    for a in accesses {
        let w = promotable_width(a.ty)?;
        if a.off + w > size {
            return None;
        }
        if !views
            .iter()
            .any(|&(o, t, tw)| o == a.off && (t == a.ty || same_bits(t, a.ty, tw, w)))
        {
            views.push((a.off, a.ty, w));
        }
    }
    let contains = |outer: &(i64, IrType, i64), inner: &(i64, IrType, i64)| {
        outer.0 <= inner.0 && inner.0 + inner.2 <= outer.0 + outer.2
    };
    // Same bytes, different type: not independent storage (rule 3).
    for (i, x) in views.iter().enumerate() {
        if views[i + 1..].iter().any(|y| x.0 == y.0 && x.2 == y.2) {
            return None;
        }
    }
    let mut fields: Vec<(i64, IrType, i64)> = Vec::new();
    let mut covering_views: Vec<(i64, IrType, i64)> = Vec::new();
    for x in &views {
        if views.iter().any(|y| y != x && contains(x, y)) {
            covering_views.push(*x);
        } else {
            fields.push(*x);
        }
    }
    fields.sort_by_key(|&(o, _, _)| o);
    // Equal offsets were refused or merged above, so comparing neighbours
    // rules out every overlap between fields.
    if fields.windows(2).any(|w2| w2[0].0 + w2[0].2 > w2[1].0) {
        return None;
    }
    let mut covering: FxHashSet<(i64, i64)> = FxHashSet::default();
    let mut carriers: Vec<(i64, IrType, i64)> = Vec::new();
    for cv in &covering_views {
        if !cv.1.is_integer() {
            return None;
        }
        let (lo, hi) = (cv.0, cv.0 + cv.2);
        let carriers_before = carriers.len();
        let mut parts = 0usize;
        let mut pos = lo;
        for f in fields.iter().filter(|f| f.0 < hi && f.0 + f.2 > lo) {
            if !contains(cv, f) || !f.1.is_integer() {
                return None;
            }
            fill_gap(&mut carriers, pos, f.0);
            pos = f.0 + f.2;
            parts += 1;
        }
        fill_gap(&mut carriers, pos, hi);
        parts += carriers.len() - carriers_before;
        if parts > MAX_COVERING_PARTS {
            return None;
        }
        covering.insert((cv.0, cv.2));
    }
    // Two covering views may share a gap; keep one carrier per offset.
    carriers.sort_by_key(|&(o, _, _)| o);
    carriers.dedup_by_key(|c| c.0);
    fields.extend(carriers);
    fields.sort_by_key(|&(o, _, _)| o);
    if fields.windows(2).any(|w2| w2[0].0 + w2[0].2 > w2[1].0) {
        return None;
    }
    Some((fields, covering))
}

/// Two integer types of one width: the same bits, read or written with a
/// different signedness.
fn same_bits(a: IrType, b: IrType, wa: i64, wb: i64) -> bool {
    wa == wb && a.is_integer() && b.is_integer()
}

/// Most parts (fields plus gap carriers) one covering view may be rebuilt
/// from.
///
/// Rebuilding a covering load from `k` parts costs `k` field values plus
/// `k - 1` shifts and `k - 1` ors (signed parts add a mask each), a covering
/// store `k - 1` shifts and `k` narrow stores, against the memory form's one
/// wide access. The memory form's hidden cost is store forwarding: a wide
/// load that spans several narrower in-flight stores cannot forward on any
/// current x86 core and waits for them to retire (~12-15 cycles on
/// Golden/Raptor Cove and Zen 4/5), while the split form keeps every part in
/// a register and lets the pack fold away entirely when its readers only
/// take parts back out (`simplify::packed_range_source`). At `k <= 4` the
/// rebuild is at most ~10 single-cycle ALU ops with a dependency depth of 3,
/// cheaper than one forwarding stall; at `k = 8` (an `I64` over eight byte
/// members) it is ~22 ops plus the register pressure of eight live parts,
/// and measurement on the regression corpus (`two_reg_return_carrier.c`
/// `check12`: +57 instructions, +4 callee-saved pushes) showed it losing.
const MAX_COVERING_PARTS: usize = 4;

/// Tile `[pos, end)` with naturally aligned integer carriers, widest first.
fn fill_gap(out: &mut Vec<(i64, IrType, i64)>, mut pos: i64, end: i64) {
    while pos < end {
        let (ty, w) = [(IrType::I64, 8i64), (IrType::I32, 4), (IrType::I16, 2)]
            .into_iter()
            .find(|&(_, w)| pos % w == 0 && pos + w <= end)
            .unwrap_or((IrType::I8, 1));
        out.push((pos, ty, w));
        pos += w;
    }
}

/// Rule 3b, load side: `dest = or(zext(f_i) << 8*rel_i)` over `parts`
/// `(relative offset, field type, field alloca)`, all strictly narrower than
/// the covering type `ty`.
///
/// A signed field is masked after its (sign-)extending cast instead of being
/// cast as unsigned: the backends keep signed narrow values in sign-extended
/// register images, so reinterpreting one as unsigned would let its sign
/// copies leak into the neighbouring field's bits. The mask is folded away
/// whenever the consumer only needs the low bits (the usual case: the
/// matching covering store or a pair extract).
fn covering_load(
    dest: Value,
    ty: IrType,
    parts: &[(i64, IrType, u32)],
    next: &mut u32,
) -> Vec<Instruction> {
    let mut fresh = || {
        let v = Value(*next);
        *next += 1;
        v
    };
    let mut out = Vec::new();
    let mut acc: Option<Operand> = None;
    for &(rel, fty, field) in parts {
        let l = fresh();
        out.push(Instruction::Load {
            volatile: false,
            dest: l,
            ptr: Value(field),
            ty: fty,
            seg_override: AddressSpace::Default,
        });
        let wide = fresh();
        out.push(Instruction::Cast {
            dest: wide,
            src: Operand::Value(l),
            from_ty: fty,
            to_ty: ty,
        });
        let mut part = Operand::Value(wide);
        if fty.is_signed() {
            let m = fresh();
            let mask = ((1u64 << (8 * ty_size(fty))) - 1) as i64;
            out.push(Instruction::BinOp {
                dest: m,
                op: IrBinOp::And,
                lhs: part,
                rhs: Operand::Const(IrConst::from_i64(mask, ty)),
                ty,
            });
            part = Operand::Value(m);
        }
        if rel > 0 {
            let sh = fresh();
            out.push(Instruction::BinOp {
                dest: sh,
                op: IrBinOp::Shl,
                lhs: part,
                rhs: Operand::Const(IrConst::from_i64(8 * rel, ty)),
                ty,
            });
            part = Operand::Value(sh);
        }
        acc = Some(match acc {
            None => part,
            Some(prev) => {
                let o = fresh();
                out.push(Instruction::BinOp {
                    dest: o,
                    op: IrBinOp::Or,
                    lhs: prev,
                    rhs: part,
                    ty,
                });
                Operand::Value(o)
            }
        });
    }
    let src = acc.unwrap_or(Operand::Const(IrConst::from_i64(0, ty)));
    out.push(Instruction::Copy { dest, src });
    out
}

/// A field access whose type is a sign variant of the field's (same width,
/// both integer; see `field_layout`): `dest = cast(load fty field)` for a
/// load, `store fty cast(val) -> field` for a store. The casts change no
/// bits, so they cost nothing once the field is a register.
fn retyped_access(inst: &Instruction, fty: IrType, field: u32, next: &mut u32) -> Vec<Instruction> {
    let v = Value(*next);
    *next += 1;
    match *inst {
        Instruction::Load { dest, ty, .. } => vec![
            Instruction::Load {
                volatile: false,
                dest: v,
                ptr: Value(field),
                ty: fty,
                seg_override: AddressSpace::Default,
            },
            Instruction::Cast {
                dest,
                src: Operand::Value(v),
                from_ty: fty,
                to_ty: ty,
            },
        ],
        Instruction::Store { val, ty, .. } => vec![
            Instruction::Cast {
                dest: v,
                src: val,
                from_ty: ty,
                to_ty: fty,
            },
            Instruction::Store {
                volatile: false,
                val: Operand::Value(v),
                ptr: Value(field),
                ty: fty,
                seg_override: AddressSpace::Default,
            },
        ],
        _ => unreachable!("access records only Load/Store"),
    }
}

/// Rule 3b, store side: one `trunc(val >> 8*rel_i)` store per field.
fn covering_store(
    val: Operand,
    ty: IrType,
    parts: &[(i64, IrType, u32)],
    next: &mut u32,
) -> Vec<Instruction> {
    let mut out = Vec::new();
    for &(rel, fty, field) in parts {
        let mut part = val;
        if rel > 0 {
            let sh = Value(*next);
            *next += 1;
            out.push(Instruction::BinOp {
                dest: sh,
                op: IrBinOp::LShr,
                lhs: part,
                rhs: Operand::Const(IrConst::from_i64(8 * rel, ty)),
                ty,
            });
            part = Operand::Value(sh);
        }
        let t = Value(*next);
        *next += 1;
        out.push(Instruction::Cast {
            dest: t,
            src: part,
            from_ty: ty,
            to_ty: fty,
        });
        out.push(Instruction::Store {
            volatile: false,
            val: Operand::Value(t),
            ptr: Value(field),
            ty: fty,
            seg_override: AddressSpace::Default,
        });
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, BlockId, IrModule, Terminator};

    fn make_func(instructions: Vec<Instruction>, ret: Option<Operand>) -> IrFunction {
        let mut func = IrFunction::new("test".to_string(), IrType::I64, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(ret),
            source_spans: Vec::new(),
        });
        func
    }

    #[test]
    fn splits_non_escaping_struct_with_ptr_and_int_fields() {
        let mut module = IrModule::new();
        module.functions.push(make_func(
            vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 16,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::Store {
                    val: Operand::Value(Value(2)),
                    ptr: Value(1),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::GetElementPtr {
                    dest: Value(3),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::I64,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::I64(42)),
                    ptr: Value(3),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Load {
                    dest: Value(4),
                    ptr: Value(3),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            Some(Operand::Value(Value(4))),
        ));
        let changed = super::super::run(&mut module);
        assert!(changed > 0, "struct with Ptr + I64 fields must split");
    }

    #[test]
    fn refuses_to_split_alloca_whose_address_is_stored_into_another_alloca() {
        let mut module = IrModule::new();
        module.functions.push(make_func(
            vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 16,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Alloca {
                    dest: Value(2),
                    ty: IrType::Ptr,
                    size: 16,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::I64(10)),
                    ptr: Value(1),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::GetElementPtr {
                    dest: Value(3),
                    base: Value(1),
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::I64,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::I64(20)),
                    ptr: Value(3),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Store {
                    val: Operand::Value(Value(1)),
                    ptr: Value(2),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::GetElementPtr {
                    dest: Value(4),
                    base: Value(2),
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::I64,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::I64(99)),
                    ptr: Value(4),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Load {
                    dest: Value(5),
                    ptr: Value(1),
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            Some(Operand::Value(Value(5))),
        ));
        super::super::run(&mut module);
        let has_v1 = module.functions[0].blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Alloca { dest: Value(1), .. }));
        assert!(
            has_v1,
            "alloca v1 whose address is stored into v2 must not be split"
        );
    }
}

#[cfg(test)]
mod copy_out_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, IrModule, Terminator};

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

    fn param(v: u32, idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(v),
            param_idx: idx,
            ty: IrType::Ptr,
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

    fn load(dest: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Load {
            volatile: false,
            dest: Value(dest),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
        }
    }

    fn store(val: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            volatile: false,
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
        }
    }

    fn memcpy(dest: u32, src: u32, size: usize) -> Instruction {
        Instruction::Memcpy {
            dest: Value(dest),
            src: Value(src),
            size,
        }
    }

    fn run(instructions: Vec<Instruction>) -> Vec<Instruction> {
        let mut f = IrFunction::new("t".into(), IrType::Void, vec![], false);
        f.next_value_id = 100;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        let mut module = IrModule::new();
        module.functions.push(f);
        super::super::run(&mut module);
        module.functions[0].blocks[0].instructions.clone()
    }

    /// `(offset, type)` of every store whose address is `base + const`.
    fn stores_into(insts: &[Instruction], base: u32) -> Vec<(i64, IrType)> {
        let mut out: Vec<(i64, IrType)> = insts
            .iter()
            .filter_map(|i| match i {
                Instruction::Store { ptr, ty, .. } => insts.iter().find_map(|g| match g {
                    Instruction::GetElementPtr {
                        dest,
                        base: b,
                        offset,
                        ..
                    } if *dest == *ptr && b.0 == base => const_i64(offset).map(|o| (o, *ty)),
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        out.sort_by_key(|&(o, _)| o);
        out
    }

    /// The zstd `HUF_DecompressFastArgs` shape: `memcpy(&local, &args->x,
    /// 32)`, field work at a constant offset, `memcpy(&args->x, &local, 32)`.
    /// The copy-out becomes one store per field, and the bytes no access
    /// names are carried, so all 32 destination bytes are still written.
    #[test]
    fn copy_in_work_copy_out_splits_and_writes_every_byte() {
        let insts = run(vec![
            alloca(1, 32),
            param(10, 0),
            param(11, 1),
            memcpy(1, 10, 32),
            gep(3, 1, 8),
            load(4, 3, IrType::I64),
            store(4, 3, IrType::I64),
            memcpy(11, 1, 32),
        ]);
        assert!(
            !insts
                .iter()
                .any(|i| matches!(i, Instruction::Memcpy { .. })),
            "both block copies are expanded: {insts:?}"
        );
        assert!(
            !insts
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { size: 32, .. })),
            "the aggregate is dropped: {insts:?}"
        );
        let offs: Vec<i64> = stores_into(&insts, 11).iter().map(|&(o, _)| o).collect();
        assert_eq!(offs, [0, 8, 16, 24], "{insts:?}");
    }

    /// Gaps are carried by the widest naturally aligned integers that fit.
    #[test]
    fn copy_out_carries_uneven_gaps() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            param(11, 1),
            memcpy(1, 10, 16),
            gep(3, 1, 2),
            load(4, 3, IrType::I16),
            store(4, 3, IrType::I16),
            memcpy(11, 1, 16),
        ]);
        assert_eq!(
            stores_into(&insts, 11),
            [
                (0, IrType::I16),
                (2, IrType::I16),
                (4, IrType::I32),
                (8, IrType::I64)
            ],
            "{insts:?}"
        );
    }

    /// A copy-out into another local object stays a block copy: the source
    /// keeps its memory (the copy-in rule may still split the destination).
    #[test]
    fn copy_out_into_a_local_keeps_the_source() {
        let insts = run(vec![
            alloca(1, 32),
            alloca(2, 32),
            param(10, 0),
            memcpy(1, 10, 32),
            gep(3, 1, 8),
            load(4, 3, IrType::I64),
            store(4, 3, IrType::I64),
            memcpy(2, 1, 32),
            gep(5, 2, 8),
            load(6, 5, IrType::I64),
            store(6, 10, IrType::I64),
        ]);
        assert!(
            insts
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { dest, .. } if dest.0 == 1)),
            "{insts:?}"
        );
    }

    /// A partial copy-out cannot be expressed through whole fields.
    #[test]
    fn partial_copy_out_refuses_the_split() {
        let insts = run(vec![
            alloca(1, 32),
            param(10, 0),
            param(11, 1),
            memcpy(1, 10, 32),
            gep(3, 1, 8),
            load(4, 3, IrType::I64),
            store(4, 3, IrType::I64),
            memcpy(11, 1, 16),
        ]);
        assert!(
            insts
                .iter()
                .any(|i| matches!(i, Instruction::Alloca { size: 32, .. })),
            "{insts:?}"
        );
        assert_eq!(
            insts
                .iter()
                .filter(|i| matches!(i, Instruction::Memcpy { .. }))
                .count(),
            2,
            "{insts:?}"
        );
    }
}

/// Rule 5b: whole-object `memset` expansion. Each test pins one acceptance
/// condition of the rule (whole object, offset 0, unused result, library
/// permission, an exact per-field splat) or the semantics of the expansion.
#[cfg(test)]
mod memset_tests {
    use super::*;
    use crate::ir::reexports::{BasicBlock, BlockId, CallInfo, IrModule, Terminator};

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

    fn param(v: u32, idx: usize) -> Instruction {
        Instruction::ParamRef {
            dest: Value(v),
            param_idx: idx,
            ty: IrType::Ptr,
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

    fn load(dest: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Load {
            volatile: false,
            dest: Value(dest),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
        }
    }

    fn store(val: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            volatile: false,
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty,
            seg_override: AddressSpace::Default,
        }
    }

    /// `memset(ptr, byte, len)`; `lib` is `CallInfo::assume_library`.
    fn memset(result: Option<u32>, ptr: u32, byte: i32, len: i64, lib: bool) -> Instruction {
        Instruction::Call {
            func: "memset".into(),
            info: CallInfo {
                dest: result.map(Value),
                args: vec![
                    Operand::Value(Value(ptr)),
                    Operand::Const(IrConst::I32(byte)),
                    Operand::Const(IrConst::I64(len)),
                ],
                arg_types: vec![IrType::Ptr, IrType::I32, IrType::I64],
                return_type: IrType::Ptr,
                assume_library: lib,
                ..CallInfo::default()
            },
        }
    }

    fn run(instructions: Vec<Instruction>) -> Vec<Instruction> {
        let mut f = IrFunction::new("t".into(), IrType::Void, vec![], false);
        f.next_value_id = 100;
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: vec![],
        });
        let mut module = IrModule::new();
        module.functions.push(f);
        super::super::run(&mut module);
        module.functions[0].blocks[0].instructions.clone()
    }

    fn has_memset(insts: &[Instruction]) -> bool {
        insts
            .iter()
            .any(|i| matches!(i, Instruction::Call { func, .. } if func == "memset"))
    }

    fn kept(insts: &[Instruction], size: usize) -> bool {
        insts
            .iter()
            .any(|i| matches!(i, Instruction::Alloca { size: s, .. } if *s == size))
    }

    /// `(type, value)` of every constant store, in program order.
    fn const_stores(insts: &[Instruction]) -> Vec<(IrType, IrConst)> {
        insts
            .iter()
            .filter_map(|i| match i {
                Instruction::Store {
                    val: Operand::Const(c),
                    ty,
                    ..
                } => Some((*ty, c.clone())),
                _ => None,
            })
            .collect()
    }

    /// The zstd `HUF_initCStream` shape: `memset(&bitC, 0, sizeof bitC)`,
    /// then field work. The call becomes one zero store per field and the
    /// aggregate is dropped.
    #[test]
    fn whole_object_memset_becomes_per_field_constant_stores() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            memset(None, 1, 0, 16, true),
            gep(3, 1, 8),
            store(10, 3, IrType::Ptr),
            load(4, 1, IrType::I64),
            load(5, 3, IrType::Ptr),
            store(4, 10, IrType::I64),
            store(5, 10, IrType::Ptr),
        ]);
        assert!(!has_memset(&insts), "the memset is expanded: {insts:?}");
        assert!(!kept(&insts, 16), "the aggregate is dropped: {insts:?}");
        let zeros = const_stores(&insts);
        assert_eq!(zeros.len(), 2, "one store per field: {insts:?}");
        assert!(
            zeros.iter().all(|(_, c)| c.to_i64() == Some(0)),
            "{zeros:?}"
        );
    }

    /// A nonzero byte is splatted over each field's width, exactly as the
    /// library would leave the bytes: 0xAB in an i32, an i8 and a pointer.
    #[test]
    fn nonzero_byte_is_splatted_per_field_width() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            memset(None, 1, 0x1AB, 16, true),
            gep(3, 1, 4),
            gep(5, 1, 8),
            load(4, 1, IrType::I32),
            load(6, 3, IrType::I8),
            load(7, 5, IrType::Ptr),
            store(4, 10, IrType::I32),
            store(6, 10, IrType::I8),
            store(7, 10, IrType::Ptr),
        ]);
        assert!(!has_memset(&insts), "{insts:?}");
        let mut got: Vec<(IrType, i64)> = const_stores(&insts)
            .into_iter()
            .map(|(t, c)| (t, c.to_i64().unwrap()))
            .collect();
        got.sort_by_key(|&(t, _)| ty_size(t));
        // The int argument is converted to `unsigned char` first: 0x1AB -> 0xAB.
        assert_eq!(
            got,
            [
                (IrType::I8, 0xABu8 as i8 as i64),
                (IrType::I32, 0xABAB_ABABu32 as i32 as i64),
                (IrType::Ptr, 0xABAB_ABAB_ABAB_ABABu64 as i64),
            ],
            "{insts:?}"
        );
    }

    /// A memset of part of the object writes bytes no field owns as a whole:
    /// the object keeps its memory and the call survives.
    #[test]
    fn partial_memset_refuses_the_split() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            memset(None, 1, 0, 8, true),
            gep(3, 1, 8),
            load(4, 1, IrType::I64),
            load(5, 3, IrType::I64),
            store(4, 10, IrType::I64),
            store(5, 10, IrType::I64),
        ]);
        assert!(has_memset(&insts) && kept(&insts, 16), "{insts:?}");
    }

    /// A memset that starts inside the object is refused even when its
    /// length would fit the tail.
    #[test]
    fn interior_memset_refuses_the_split() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            gep(3, 1, 8),
            memset(None, 3, 0, 8, true),
            load(4, 1, IrType::I64),
            load(5, 3, IrType::I64),
            store(4, 10, IrType::I64),
            store(5, 10, IrType::I64),
        ]);
        assert!(has_memset(&insts) && kept(&insts, 16), "{insts:?}");
    }

    /// `memset` returns its pointer; a used result would name the dropped
    /// object, so the call (and the object) must stay.
    #[test]
    fn memset_with_used_result_refuses_the_split() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            memset(Some(20), 1, 0, 16, true),
            store(20, 10, IrType::Ptr),
            gep(3, 1, 8),
            load(4, 1, IrType::I64),
            load(5, 3, IrType::I64),
            store(4, 10, IrType::I64),
            store(5, 10, IrType::I64),
        ]);
        assert!(has_memset(&insts) && kept(&insts, 16), "{insts:?}");
    }

    /// Without `assume_library` (`-fno-builtin`, `-ffreestanding`, a TU
    /// definition of `memset`) the call is opaque and escapes the object.
    #[test]
    fn non_library_memset_refuses_the_split() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            memset(None, 1, 0, 16, false),
            gep(3, 1, 8),
            load(4, 1, IrType::I64),
            load(5, 3, IrType::I64),
            store(4, 10, IrType::I64),
            store(5, 10, IrType::I64),
        ]);
        assert!(has_memset(&insts) && kept(&insts, 16), "{insts:?}");
    }

    /// Float fields take byte 0 (+0.0); any other byte would be an arbitrary
    /// bit pattern, so the object is refused.
    #[test]
    fn float_fields_admit_only_the_zero_byte() {
        let body = |byte: i32| {
            vec![
                alloca(1, 16),
                param(10, 0),
                memset(None, 1, byte, 16, true),
                gep(3, 1, 8),
                load(4, 1, IrType::F64),
                load(5, 3, IrType::I64),
                store(4, 10, IrType::F64),
                store(5, 10, IrType::I64),
            ]
        };
        let zero = run(body(0));
        assert!(!has_memset(&zero) && !kept(&zero, 16), "{zero:?}");
        assert!(
            const_stores(&zero)
                .iter()
                .any(|(t, c)| *t == IrType::F64
                    && matches!(c, IrConst::F64(x) if x.to_bits() == 0)),
            "+0.0 in the float field: {zero:?}"
        );
        let ones = run(body(1));
        assert!(has_memset(&ones) && kept(&ones, 16), "{ones:?}");
    }

    /// memset, one field write, copy-out: every destination byte the block
    /// copy wrote must still be written, so the carriers receive the memset
    /// bytes too (a lost carrier would leave the destination's old bytes).
    #[test]
    fn memset_feeds_copy_out_carriers() {
        let insts = run(vec![
            alloca(1, 16),
            param(10, 0),
            param(11, 1),
            memset(None, 1, 0, 16, true),
            store(10, 1, IrType::I32),
            load(12, 1, IrType::I32),
            store(12, 11, IrType::I32),
            Instruction::Memcpy {
                dest: Value(11),
                src: Value(1),
                size: 16,
            },
        ]);
        assert!(!has_memset(&insts) && !kept(&insts, 16), "{insts:?}");
        // Field I32@0 plus carriers I32@4 and I64@8: three zero stores.
        assert_eq!(const_stores(&insts).len(), 3, "{insts:?}");
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    fn acc(off: i64, ty: IrType, is_load: bool) -> Access {
        Access {
            block: 0,
            index: 0,
            is_load,
            off,
            ty,
        }
    }

    fn offsets(fields: &[(i64, IrType, i64)]) -> Vec<(i64, i64)> {
        fields.iter().map(|&(o, _, w)| (o, w)).collect()
    }

    #[test]
    fn two_register_carrier_over_three_u32_members() {
        // {u32 a, b, c} written per member, read back as I64 @0 + U32 @8.
        let a = [
            acc(0, IrType::U32, false),
            acc(4, IrType::U32, false),
            acc(8, IrType::U32, false),
            acc(0, IrType::I64, true),
            acc(8, IrType::U32, true),
        ];
        let (fields, covering) = field_layout(&a, 12).expect("splits");
        assert_eq!(offsets(&fields), vec![(0, 4), (4, 4), (8, 4)]);
        assert!(covering.contains(&(0, 8)));
        assert!(!covering.contains(&(8, 4)), "an exact view is a field");
    }

    #[test]
    fn sign_variants_are_one_view_typed_by_the_first_access() {
        // Zero-fill tail `I32 @8` against the carrier's `U32 @8` load.
        let a = [
            acc(0, IrType::I64, false),
            acc(8, IrType::I32, false),
            acc(0, IrType::I32, false),
            acc(4, IrType::I32, false),
            acc(8, IrType::I16, false),
            acc(10, IrType::I8, false),
            acc(0, IrType::I64, true),
            acc(8, IrType::U32, true),
        ];
        let (fields, covering) = field_layout(&a, 12).expect("splits");
        assert_eq!(
            offsets(&fields),
            vec![(0, 4), (4, 4), (8, 2), (10, 1), (11, 1)],
            "c, d and a carrier for the padding byte"
        );
        assert!(covering.contains(&(0, 8)) && covering.contains(&(8, 4)));
    }

    #[test]
    fn same_bytes_with_a_non_integer_type_still_refuse() {
        for (x, y) in [(IrType::I32, IrType::F32), (IrType::I64, IrType::Ptr)] {
            let a = [
                acc(0, x, false),
                acc(0, y, true),
                acc(8, IrType::I32, false),
            ];
            assert!(field_layout(&a, 12).is_none(), "{x:?} vs {y:?}");
        }
    }

    #[test]
    fn covering_views_must_be_integer_over_whole_integer_fields() {
        // A float covering view needs a bit cast, not a shift.
        let a = [
            acc(0, IrType::I32, false),
            acc(4, IrType::I32, false),
            acc(0, IrType::F64, true),
        ];
        assert!(field_layout(&a, 8).is_none());
        // A field straddling the covering view's edge.
        let a = [
            acc(2, IrType::I32, false),
            acc(0, IrType::I32, true),
            acc(6, IrType::I16, false),
        ];
        assert!(field_layout(&a, 8).is_none());
        // A Ptr member outside every covering view is an ordinary field.
        let a = [
            acc(0, IrType::Ptr, false),
            acc(8, IrType::I64, false),
            acc(8, IrType::I32, true),
        ];
        assert!(field_layout(&a, 16).is_some());
        let a = [
            acc(0, IrType::I32, false),
            acc(4, IrType::F32, false),
            acc(0, IrType::I64, true),
        ];
        assert!(
            field_layout(&a, 8).is_none(),
            "float member under a covering view"
        );
    }

    #[test]
    fn covering_part_count_is_capped() {
        // Eight byte members under one I64: past MAX_COVERING_PARTS.
        let mut a: Vec<Access> = (0..8).map(|o| acc(o, IrType::U8, false)).collect();
        a.push(acc(0, IrType::I64, true));
        assert!(field_layout(&a, 8).is_none());
        // Four u16 members are at the cap and split.
        let mut a: Vec<Access> = (0..4).map(|o| acc(2 * o, IrType::U16, false)).collect();
        a.push(acc(0, IrType::I64, true));
        let (fields, _) = field_layout(&a, 8).expect("4 parts split");
        assert_eq!(fields.len(), 4);
        // Gap carriers count: {u8 @0, u8 @7} tiles [1, 7) as I8 @1, I16 @2,
        // I16 @4, I8 @6, six parts in all.
        let a = [
            acc(0, IrType::U8, false),
            acc(7, IrType::U8, false),
            acc(0, IrType::I64, true),
        ];
        assert!(
            field_layout(&a, 8).is_none(),
            "carriers push it past the cap"
        );
    }

    #[test]
    fn gaps_get_naturally_aligned_carriers() {
        // {u32 @0, u8 @7} under I64: [4, 7) is I16 @4 + I8 @6 (an I32 @4
        // would overrun the member at 7).
        let a = [
            acc(0, IrType::U32, false),
            acc(7, IrType::U8, false),
            acc(0, IrType::I64, true),
        ];
        let (fields, _) = field_layout(&a, 8).expect("4 parts split");
        assert_eq!(offsets(&fields), vec![(0, 4), (4, 2), (6, 1), (7, 1)]);
    }
}
