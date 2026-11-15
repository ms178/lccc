//! Lane-parallel ARX vectorization for block-cipher round loops.
//!
//! # What this pass recognizes
//!
//! ARX permutations (Add-Rotate-Xor: ChaCha20, Salsa20, BLAKE rounds) keep
//! their state in a small `u32 x[4k]` array (in practice 16 words) that
//! survives as ONE alloca addressed through constant-offset GEPs — because
//! some other part of the function (the initial copy loop or the final
//! feed-forward loop) walks the same array with a variable offset, which
//! blocks aggregate promotion. The round loop's body is then a long run of
//!
//! ```text
//! Load  a ← x[s]        Load b ← x[s']       BinOp Add/Xor
//! Store x[s] ← result   (per statement)      rotate idioms
//! ```
//!
//! scalar statements: four *lane-isomorphic* statement streams interleaved
//! with stride `K` (the ops per quarter-round). Packed as
//!
//! ```text
//! V[g] = ( x[4g+0], x[4g+1], x[4g+2], x[4g+3] )   // 4×I32 xmm register
//! ```
//!
//! every group of four isomorphic ops becomes ONE vector instruction:
//!
//! * `a += b`   → `paddd`     * `d ^= a`  → `pxor`
//! * `rol(d,n)` → `pslld + psrld + por` (the SSE2 rotate triple), except
//!   `n = 16`: a 32-bit lane rotate by half a lane is an intra-dword swap of
//!   the two 16-bit halves, so the backend folds it to `pshuflw $0xb1` +
//!   `pshufhw $0xb1` — two 1-cycle shuffles with no `%xmm0` staging and no
//!   second live temporary (see `emit_int_rotl_i32x4`)
//!
//! The diagonal round reads its b/c/d operands from *lane-rotated* frames
//! (`QR_l` uses `x[4h + (l+r_h)%4]`); those frames materialize with one
//! `pshufd` each and rotate back on the next round's reads — the classic
//! SSE2 single-block ChaCha20 shuffle trick (6 `pshufd` per double round),
//! derived here mechanically from the slot pattern instead of hard-coded.
//!
//! # Lane frames are chosen for latency, not op count
//!
//! Which role gets shuffled is a free choice: lane `l` of a chunk may
//! compute statement `(l + s) % 4` for any shift `s`, as long as every
//! chunk linked by a direct op→op edge uses the same `s` (those values
//! never pass through a slot, so they cannot be realigned).  Values that
//! cross a slot are realigned exactly by `rotated`, whatever the frames.
//! So each op→op component picks its own `s`, and the choice matters: a
//! round loop is a serial recurrence (ChaCha's blocks even chain through
//! the feed-forward), so its speed is the critical-path latency, not the
//! op count.  Statement 0's frame (`s = 0`) shuffles the diagonal round's
//! `b` role, and `b = rotl(b, 7)` feeds `a += b` immediately, putting two
//! `pshufd` per double round on the critical path (30 cycles vs 28).
//! Anchoring on `b` instead shuffles `a`, `c` and `d`, which all have
//! slack there.  Rather than hard-coding that cipher fact, the emitter
//! simulates the four shifts of each component, in program order, and
//! keeps the one whose results are ready earliest (ties: fewer emitted
//! instructions, then `s = 0`, so frame-insensitive kernels are
//! unchanged).  Latencies: every emitted op is 1 cycle except the
//! shift-pair rotate (2; 1 with AVX-512VL `vprold`), uniform across Zen
//! 3/4/5 and Intel P-cores.  `scripts/loop_latency.py` checks the result.
//!
//! # Soundness model (fail-closed everywhere)
//!
//! * The body block must consist ONLY of: constant GEPs on the one state
//!   base, u32 Loads/Stores through those GEPs, u32 `Add`/`Xor` BinOps,
//!   rotate idioms (`Shl/LShr/Or` triples — folded to vector rotates; the
//!   canonical `RotateLeft` op is also accepted), and IV maintenance (any
//!   non-u32 `Add`, `Cmp`, `Phi`, `Copy`, `Branch`). A u32 ARX value can
//!   never flow from a non-u32 instruction without a `Cast`, and any
//!   unknown value flowing INTO an ARX operand fails the extraction — so
//!   the transparency list is fail-closed by construction.
//! * Every operand is bound at extraction to a **versioned slot snapshot**
//!   (`Slot{slot, version}`, version = the number of stores to that slot so
//!   far in the body) or to the defining op (`Op{index}`). Vector emission
//!   resolves each operand exactly: a snapshot at version `v` reads the
//!   `v`-th group write's result rotated so lane `l` carries the reading
//!   statement's slot — a load executed before a store never sees the
//!   store's value, exactly like the scalar code.
//! * Chunks are verified op-by-op: identical opcode, identical constant,
//!   identical store pattern, slot lanes advancing by +1 per statement,
//!   snapshot versions equal across the four statements, and operand op
//!   indices advancing by exactly `K` (same op position, next QR). The
//!   first failure rejects the loop.
//! * No consumed value may escape the body block (used by a surviving
//!   instruction, a header phi, or a terminator anywhere) — the rewrite
//!   would delete its definition otherwise. Checked exhaustively.
//! * Loop mode re-materializes the state at every loop exit through a
//!   `Phi` (when the exit block merges loop and bypass edges) or direct
//!   stores (single-predecessor exits), so exit paths read exactly what
//!   the scalar code would have left in the array; when the loop body
//!   never ran, the preheader's `VecLoad` value is stored back unchanged.
//!   Exit-block predecessors outside the loop must be dominated by the
//!   preheader (V_init live-in) — otherwise the transform bails.
//!
//! # Boundary absorption (entry copy and feed-forward)
//!
//! The round loop rarely travels alone: ChaCha-style kernels copy the
//! caller's input into the state first (`x[i] = in[i]`) and feed it
//! forward at the end (`out[i] = x[i] + in[i]`).  Left to the general
//! vectorizer, both loops stage 16-byte vector traffic through the
//! state alloca that the pack/exit immediately re-touch with scalar
//! accesses — the exact store-to-load-forwarding shape that cratered
//! chacha20_block on Zen 5 (see `match_entry_copy`).  Two fail-closed
//! matchers absorb them:
//!
//! * `match_entry_copy`: the preheader pack reads `in` directly with
//!   per-element 4-byte loads and the copy loop is deleted.
//! * `match_exit_combine`: the exit computes `out_g = V_exit_g +
//!   vecload(in)_g` — the state straight from registers, the input via
//!   wide loads that are plain L1 hits by exit time — behind a
//!   one-sided runtime overlap guard (scalar fallback kept cold; see
//!   `install_exit_materialization`).
//!   When nothing else reads the state array, its spill stores move to
//!   the fallback path and the hot exit touches memory only to store
//!   `out`.
//!
//! # Placement
//!
//! Runs BEFORE loop unrolling (Phase 2b, `-O2+`, x86-64, not `-Os/-Oz`):
//! the transform keeps the round loop *rolled* — one ~46-instruction vector
//! body executed `trip` times, which is the shape ICX emits and beats every
//! unrolled scalar form on both instruction count and I-cache. The
//! unrollers refuse loops containing this pass's marker intrinsics
//! (`VecRotlI32x4`/`VecShufdI32x4`), so the rolled shape survives every
//! later unroll phase. The rotate idioms are folded *inside* this pass
//! (not by re-running bit_idioms early) so the pipeline order — and every
//! other function's codegen — is untouched.
//!
//! Kill switch: `CCC_DISABLE_PASSES="vec_arx"` (the standard dispatcher
//! knob).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::instruction::{BasicBlock, Instruction, Terminator};
use crate::ir::ops::{IrBinOp, IrCmpOp};
use crate::ir::reexports::IrFunction;
use crate::ir::reexports::{BlockId, IntrinsicOp, IrConst, Operand, Value};
use crate::passes::loop_analysis::{self, DominanceChecker};

/// Lanes packed per vector (4×I32 in one 128-bit register).
const LANES: usize = 4;

/// One scalar ARX op of the body, with operands bound to versioned slot
/// snapshots or to defining ops.
#[derive(Clone, Debug)]
struct ArxOp {
    /// Instruction index in the body block (for removal).
    inst_idx: usize,
    /// The BinOp opcode (Add/Xor/RotateLeft only).
    op: IrBinOp,
    a: Arg,
    b: Arg,
    /// Store slot if this op's result is stored to the state array.
    store: Option<u8>,
    /// Result value id.
    dest: Value,
}

#[derive(Clone, Debug, PartialEq)]
enum Arg {
    /// The content of `slot` as of its `version`-th write (0 = loop-entry
    /// state).
    Slot { slot: u8, version: u32 },
    /// The result of body op `index`.
    Op { index: usize },
    /// Constant operand (only legal as a rotate amount here).
    Const(IrConst),
}

impl Arg {
    fn as_op(&self) -> Option<usize> {
        match self {
            Arg::Op { index } => Some(*index),
            _ => None,
        }
    }
}

/// pshufd immediate so destination lane `l` reads source lane `(l+r)%4`.
/// r=1 → 0x39, r=2 → 0x4E, r=3 → 0x93, r=0 → 0xE4 (identity).
fn pshufd_imm(r: u8) -> i32 {
    let r = r % 4;
    let sel = |l: u8| ((l + r) % 4) as i32;
    (sel(3) << 6) | (sel(2) << 4) | (sel(1) << 2) | sel(0)
}

/// Discover the state array domain: the base value whose constant-offset
/// GEPs the block's u32 Load/Store traffic flows through, and the slot
/// count (a multiple of 4, all slots 0..n addressable). Variable-offset
/// GEPs on the base elsewhere in the function are fine (they belong to
/// the copy/feed-forward loops, which this transform does not touch); the
/// BODY's own accesses must all resolve through constant GEPs — the
/// extractor enforces that by failing on any pointer it cannot classify.
fn discover_slot_domain(func: &IrFunction, block_idx: usize) -> Option<(Value, u8)> {
    let mut const_offs: FxHashMap<u32, FxHashSet<i64>> = FxHashMap::default();
    let mut gep_base: FxHashMap<u32, u32> = FxHashMap::default();
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Instruction::GetElementPtr {
                dest, base, offset, ..
            } = inst
            {
                if let Operand::Const(c) = offset {
                    if let Some(off) = c.to_i64() {
                        const_offs.entry(base.0).or_default().insert(off);
                    }
                }
                gep_base.insert(dest.0, base.0);
            }
        }
    }
    let block = &func.blocks[block_idx];
    let mut base_hits: FxHashMap<u32, usize> = FxHashMap::default();
    let mut traffic = 0usize;
    for inst in &block.instructions {
        let (ptr, ty) = match inst {
            Instruction::Load { ptr, ty, .. } => (ptr, ty),
            Instruction::Store { ptr, ty, .. } => (ptr, ty),
            _ => continue,
        };
        if *ty != IrType::U32 {
            continue;
        }
        let root = gep_base.get(&ptr.0).copied().unwrap_or(ptr.0);
        // A DIRECT access on the base (no GEP: `x[0]` lowers to the alloca
        // pointer itself) is slot 0. Register offset 0 so the "every slot
        // addressable" completeness check below counts it — otherwise a
        // state array whose element 0 is only ever touched directly (the
        // canonical ChaCha `x[0] += x[1]...` shape) fails discovery even
        // though every slot is provably reachable.
        if root == ptr.0 {
            const_offs.entry(root).or_default().insert(0);
        }
        *base_hits.entry(root).or_insert(0) += 1;
        traffic += 1;
    }
    if traffic < 16 {
        return None;
    }
    let (&root, _) = base_hits.iter().max_by_key(|e| e.1)?;
    let offs = const_offs.get(&root)?;
    let mut max_off: i64 = -1;
    for &o in offs {
        if o < 0 || o % 4 != 0 {
            return None;
        }
        max_off = max_off.max(o);
    }
    let nslots = ((max_off / 4) + 1) as u8;
    if nslots % LANES as u8 != 0 || nslots < LANES as u8 || nslots > 64 {
        return None;
    }
    for s in 0..nslots {
        if !offs.contains(&(4 * s as i64)) {
            return None; // every slot must be addressable
        }
    }
    Some((Value(root), nslots))
}

/// Extraction result: the flat op list plus the consumed instruction set.
struct Body {
    ops: Vec<ArxOp>,
    /// Instruction indices consumed by the ops (BinOps, loads, stores,
    /// rotate-idiom parts).
    consumed: FxHashSet<usize>,
}

/// Fold `Or(Shl(x, n), LShr(x, 32-n))` (either operand order) into a single
/// `RotateLeft(x, n)` op: the Or's slot becomes the rotate's, the Shl/LShr
/// parts are removed. Valid only when each Shl/LShr has exactly ONE consumer
/// (the Or itself) — otherwise removing them would orphan other users.
/// Mirrors bit_idioms' matcher preconditions (u32, LShr form, complementary
/// constants) so a later bit_idioms run would have produced the same op.
fn fold_rotate_idioms(ops: Vec<ArxOp>) -> Vec<ArxOp> {
    // Pass 1: single-use check + pattern match.
    // refcount[op_index] = number of Arg::Op references to that index.
    let mut refcount = vec![0usize; ops.len()];
    for op in &ops {
        for arg in [&op.a, &op.b] {
            if let Arg::Op { index } = arg {
                refcount[*index] += 1;
            }
        }
    }
    // or_idx -> (shl_idx, lshr_idx, n, shared source arg)
    let mut folds: FxHashMap<usize, (usize, usize, u8, Arg)> = FxHashMap::default();
    for (i, op) in ops.iter().enumerate() {
        if op.op != IrBinOp::Or {
            continue;
        }
        let (Some(m), Some(m2)) = (op.a.as_op(), op.b.as_op()) else {
            continue;
        };
        for (x, y) in [(m, m2), (m2, m)] {
            let (shl, lshr) = (&ops[x], &ops[y]);
            if shl.op != IrBinOp::Shl || lshr.op != IrBinOp::LShr {
                continue;
            }
            if shl.a != lshr.a {
                continue;
            }
            let (Arg::Const(cn), Arg::Const(cn2)) = (&shl.b, &lshr.b) else {
                continue;
            };
            let (Some(n), Some(n2)) = (cn.to_i64(), cn2.to_i64()) else {
                continue;
            };
            if !(1..=31).contains(&n) || n + n2 != 32 {
                continue;
            }
            // Exactly one use each (the Or) — removal orphans nobody.
            if refcount[x] != 1 || refcount[y] != 1 {
                continue;
            }
            folds.insert(i, (x, y, n as u8, shl.a.clone()));
            break;
        }
    }
    if folds.is_empty() {
        return ops;
    }
    // Pass 2: rebuild in one sweep — drop the removed parts, rewrite the
    // Or into a RotateLeft over the shared source, and remap every
    // Arg::Op reference to its new index as we go (uses only reference
    // earlier indices in block order, so new_index is already populated).
    let removed: FxHashSet<usize> = folds
        .values()
        .flat_map(|&(shl, lshr, _, _)| [shl, lshr])
        .collect();
    let mut new_index: Vec<Option<usize>> = vec![None; ops.len()];
    let mut out: Vec<ArxOp> = Vec::with_capacity(ops.len());
    for (i, mut op) in ops.into_iter().enumerate() {
        if removed.contains(&i) {
            continue;
        }
        if let Some(&(_, _, n, ref src)) = folds.get(&i) {
            op.op = IrBinOp::RotateLeft;
            op.a = src.clone();
            op.b = Arg::Const(IrConst::I32(n as i32));
        }
        let remap = |arg: &mut Arg| {
            if let Arg::Op { index } = arg {
                if let Some(ni) = new_index[*index] {
                    *index = ni;
                }
            }
        };
        remap(&mut op.a);
        remap(&mut op.b);
        new_index[i] = Some(out.len());
        out.push(op);
    }
    out
}

/// Extract the flat op list from one block (fail-closed). Transparent
/// instructions: GEPs, Phis (loop machinery), Cmps, non-u32 BinOps (IV
/// arithmetic — a u32 ARX operand can never flow from them without a
/// Cast, which is rejected), Copies with unbound sources.
fn extract_body(func: &IrFunction, block_idx: usize, base: Value, nslots: u8) -> Option<Body> {
    // ptr value id → slot (function-wide: GEPs may be hoisted).
    let mut ptr_slot: FxHashMap<u32, u8> = FxHashMap::default();
    ptr_slot.insert(base.0, 0);
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Instruction::GetElementPtr { dest, offset, .. } = inst {
                if let Operand::Const(c) = offset {
                    if let Some(off) = c.to_i64() {
                        if off >= 0 && off % 4 == 0 {
                            let s = (off / 4) as u8;
                            if s < nslots {
                                ptr_slot.insert(dest.0, s);
                            }
                        }
                    }
                }
            }
        }
    }

    let block = &func.blocks[block_idx];
    let mut val_arg: FxHashMap<u32, Arg> = FxHashMap::default();
    let mut slot_version = vec![0u32; nslots as usize];
    let mut ops: Vec<ArxOp> = Vec::new();
    let mut consumed: FxHashSet<usize> = FxHashSet::default();
    // value id → op index for store association.
    let mut pending: FxHashMap<u32, usize> = FxHashMap::default();

    for (i, inst) in block.instructions.iter().enumerate() {
        match inst {
            Instruction::GetElementPtr { .. }
            | Instruction::Phi { .. }
            | Instruction::Cmp { .. } => {}
            Instruction::Load {
                dest,
                ptr,
                ty,
                volatile,
                ..
            } => {
                if *volatile || *ty != IrType::U32 {
                    return None;
                }
                let s = ptr_slot.get(&ptr.0).copied()?;
                val_arg.insert(
                    dest.0,
                    Arg::Slot {
                        slot: s,
                        version: slot_version[s as usize],
                    },
                );
                consumed.insert(i);
            }
            Instruction::Store {
                val,
                ptr,
                ty,
                volatile,
                ..
            } => {
                if *volatile || *ty != IrType::U32 {
                    return None;
                }
                let s = ptr_slot.get(&ptr.0).copied()?;
                let op_idx = match val {
                    Operand::Value(v) => pending.get(&v.0).copied(),
                    _ => None,
                }?;
                ops[op_idx].store = Some(s);
                slot_version[s as usize] += 1;
                consumed.insert(i);
            }
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
                ..
            } => {
                if *ty != IrType::U32 {
                    continue; // IV maintenance: transparent (see doc header)
                }
                match op {
                    IrBinOp::Add
                    | IrBinOp::Xor
                    | IrBinOp::Shl
                    | IrBinOp::LShr
                    | IrBinOp::Or
                    | IrBinOp::RotateLeft => {}
                    _ => return None,
                }
                let a = operand_arg(lhs, &val_arg)?;
                let b = operand_arg(rhs, &val_arg)?;
                if *op == IrBinOp::RotateLeft {
                    match &b {
                        Arg::Const(c) => {
                            let v = c.to_i64()?;
                            if !(1..=31).contains(&v) {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                let idx = ops.len();
                pending.insert(dest.0, idx);
                val_arg.insert(dest.0, Arg::Op { index: idx });
                ops.push(ArxOp {
                    inst_idx: i,
                    op: *op,
                    a,
                    b,
                    store: None,
                    dest: *dest,
                });
                consumed.insert(i);
            }
            Instruction::Copy { dest, src } => {
                // Relay copies of bound values alias their source.
                match src {
                    Operand::Value(v) => {
                        if let Some(a) = val_arg.get(&v.0).cloned() {
                            val_arg.insert(dest.0, a);
                        }
                        // else: unbound source (IV) — dest stays unbound;
                        // any ARX use of it fails closed below.
                    }
                    Operand::Const(c) => {
                        val_arg.insert(dest.0, Arg::Const(c.clone()));
                    }
                }
            }
            _ => return None,
        }
    }
    if ops.len() < 16 {
        return None;
    }
    let ops = fold_rotate_idioms(ops);
    if ops.len() < 16 {
        return None;
    }
    Some(Body { ops, consumed })
}

fn operand_arg(op: &Operand, val_arg: &FxHashMap<u32, Arg>) -> Option<Arg> {
    match op {
        Operand::Value(v) => val_arg.get(&v.0).cloned(),
        Operand::Const(c) => Some(Arg::Const(c.clone())),
    }
}

/// A chunk: four ops (same op position across four consecutive QRs).
#[derive(Clone, Debug)]
struct Chunk {
    op_indices: [usize; LANES],
    op: IrBinOp,
    /// Statement 0's store slot (None for intermediate ops).
    store: Option<u8>,
}

/// Try to chunk the op stream with ops-per-QR = K. Verifies the full
/// isomorphism contract; returns chunks in emission order.
fn try_chunk(body: &Body, k: usize) -> Option<Vec<Chunk>> {
    let n = body.ops.len();
    if k < 4 || n % (4 * k) != 0 {
        return None;
    }
    let qrsets = n / (4 * k);
    if qrsets == 0 {
        return None;
    }
    let mut chunks = Vec::with_capacity(qrsets * k);
    for qrset in 0..qrsets {
        for j in 0..k {
            let base = 4 * k * qrset;
            let idx: [usize; LANES] = [base + j, base + k + j, base + 2 * k + j, base + 3 * k + j];
            let op0 = &body.ops[idx[0]];
            // All four lanes must be the same opcode, and every lane's
            // store (when present) must sit in the same slot GROUP with
            // the lane index advancing exactly +1 (mod 4) between
            // CONSECUTIVE lanes — comparing against lane 0 instead would
            // reject the legal lane-3 (offset +3) and accept nothing but
            // degenerate patterns.
            let lane_ok = |l: usize| -> bool {
                let a = &body.ops[idx[l]];
                let b = &body.ops[idx[l + 1]];
                if a.op != b.op {
                    return false;
                }
                match (a.store, b.store) {
                    (None, None) => {}
                    (Some(s0), Some(s1)) => {
                        if s0 / LANES as u8 != s1 / LANES as u8 {
                            return false;
                        }
                        if ((s1 % 4) + 4 - (s0 % 4)) % 4 != 1 {
                            return false;
                        }
                    }
                    _ => return false,
                }
                args_isomorphic(&a.a, &b.a, k) && args_isomorphic(&a.b, &b.b, k)
            };
            if (0..LANES - 1).any(|l| !lane_ok(l)) {
                return None;
            }
            chunks.push(Chunk {
                op_indices: idx,
                op: op0.op,
                store: op0.store,
            });
        }
    }
    Some(chunks)
}

fn args_isomorphic(a: &Arg, b: &Arg, k: usize) -> bool {
    match (a, b) {
        (
            Arg::Slot {
                slot: s0,
                version: v0,
            },
            Arg::Slot {
                slot: s1,
                version: v1,
            },
        ) => s0 / LANES as u8 == s1 / LANES as u8 && ((s1 % 4) + 4 - (s0 % 4)) % 4 == 1 && v0 == v1,
        (Arg::Op { index: m0 }, Arg::Op { index: m1 }) => m1 - m0 == k,
        (Arg::Const(c0), Arg::Const(c1)) => c0 == c1,
        _ => false,
    }
}

/// Emission context.  `Clone` so the lane-frame choice can simulate each
/// candidate shift on a scratch copy before emitting for real.
#[derive(Clone)]
struct Emitter {
    /// op index → chunk result value.
    op_result: Vec<Value>,
    /// Per group: the write results (value, rotation) in write order.
    group_writes: Vec<Vec<(Value, u8)>>,
    /// (src value, src rotation, read lane) → rotated value cache.
    rot_cache: FxHashMap<(u32, u8, u8), Value>,
    /// Whole-byte rotate amount → preheader-materialised pshufb mask
    /// value (SSSE3+): one `vpshufb` per rotate instead of the
    /// `vpslld/vpsrld/vpor` triple.  ChaCha's 16- and 8-bit rotates are
    /// byte multiples; 12 and 7 keep the triple.
    rot_masks: FxHashMap<i64, Value>,
    out: Vec<Instruction>,
    next_val: u32,
    /// Lane shift of the chunk being emitted: lane `l` computes statement
    /// `(l + shift) % 4` (see "Lane frames are chosen for latency").
    shift: u8,
    /// Value id → cycle its result is ready, with the loop-entry state
    /// ready at 0 (the latency model of the frame choice).
    ready: FxHashMap<u32, u32>,
    /// Latency of a non-byte-multiple rotate: 2 for the AVX2 shift pair
    /// (`vpslld`/`vpsrld` in parallel, then `vpor`), 1 for `vprold`.
    rot_latency: u32,
}

/// The 4-per-dword pshufb mask for a left rotation by `k` whole bytes
/// per 32-bit lane (k = 1..=3), packed as four little-endian I32 lanes.
/// pshufb semantics: dest byte i = src byte mask[i]; a left byte
/// rotation by k needs mask[4d+j] = 4d + (j + 4 - k) % 4 (verified
/// against hardware: [3,0,1,2] per dword for k=1 — the reversed index
/// order would rotate in the wrong direction).
fn byterot_mask_dwords(k: u8) -> [i32; 4] {
    let mut lanes = [0i32; 4];
    for lane in 0..4usize {
        let mut dw: i32 = 0;
        for j in 0..4usize {
            let src_byte = (4 * lane + (j + 4 - k as usize) % 4) as i32;
            dw |= src_byte << (8 * j);
        }
        lanes[lane] = dw;
    }
    lanes
}

impl Emitter {
    fn fresh(&mut self) -> Value {
        let v = Value(self.next_val);
        self.next_val += 1;
        v
    }

    fn ready_of(&self, v: Value) -> u32 {
        self.ready.get(&v.0).copied().unwrap_or(0)
    }

    /// `src` (materialized at rotation `src_rot`) rotated so lane `l`
    /// reads slot `4g + (read_lane + l) % 4`.
    fn rotated(&mut self, src: Value, src_rot: u8, read_lane: u8) -> Value {
        let delta = (read_lane + 4 - src_rot) % 4;
        if delta == 0 {
            return src;
        }
        if let Some(&v) = self.rot_cache.get(&(src.0, src_rot, read_lane)) {
            return v;
        }
        let dest = self.fresh();
        self.out.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: IntrinsicOp::VecShufdI32x4,
            dest_ptr: None,
            args: vec![
                Operand::Value(src),
                Operand::Const(IrConst::I32(pshufd_imm(delta))),
            ],
        });
        self.rot_cache.insert((src.0, src_rot, read_lane), dest);
        let lat = arx_vec_op_latency(IntrinsicOp::VecShufdI32x4, self.rot_latency);
        self.ready.insert(dest.0, self.ready_of(src) + lat);
        dest
    }

    /// Emit one chunk (four isomorphic statements) as one vector op in the
    /// current lane frame.  `None` = the chunk cannot be expressed (the
    /// caller declines the whole loop).
    fn emit_chunk(
        &mut self,
        body: &Body,
        chunk: &Chunk,
        entry: &[Value],
        nslots: u8,
    ) -> Option<Value> {
        let op0 = &body.ops[chunk.op_indices[0]];
        let is_rot = op0.op == IrBinOp::RotateLeft;
        let a = self.resolve_arg(&op0.a, entry, nslots)?;
        // A rotate's `b` is the constant AMOUNT (extract_body enforces
        // 1..=31), never a vector operand; `resolve_arg` refuses Const
        // args by design, so it must not be consulted for it.
        let b = if is_rot {
            None
        } else {
            Some(self.resolve_arg(&op0.b, entry, nslots)?)
        };
        let operand_values: Vec<Value> = std::iter::once(a).chain(b).collect();
        let dest = self.fresh();
        // Whole-byte rotate with a materialised mask: one vpshufb
        // (args = [data, mask]) instead of the shift triple.
        let shufb_mask = if is_rot {
            match &op0.b {
                Arg::Const(c) => self.rot_masks.get(&c.to_i64()?).copied(),
                _ => return None,
            }
        } else {
            None
        };
        let intrinsic_op = match op0.op {
            IrBinOp::Add => IntrinsicOp::VecAddI32x4,
            IrBinOp::Xor => IntrinsicOp::VecXorI32x4,
            IrBinOp::RotateLeft => {
                if shufb_mask.is_some() {
                    IntrinsicOp::VecShufbI32x4
                } else {
                    IntrinsicOp::VecRotlI32x4
                }
            }
            _ => return None,
        };
        let mut args = vec![Operand::Value(a)];
        if let Some(mask_v) = shufb_mask {
            args.push(Operand::Value(mask_v));
        } else if is_rot {
            if let Arg::Const(c) = &op0.b {
                args.push(Operand::Const(c.clone()));
            } else {
                return None;
            }
        } else {
            args.push(Operand::Value(b.expect("non-rotate has vector b")));
        }
        self.out.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: intrinsic_op,
            dest_ptr: None,
            args,
        });
        let lat = arx_vec_op_latency(intrinsic_op, self.rot_latency);
        let ready = operand_values
            .iter()
            .map(|&v| self.ready_of(v))
            .max()
            .unwrap_or(0)
            + lat;
        self.ready.insert(dest.0, ready);
        for &m in &chunk.op_indices {
            self.op_result[m] = dest;
        }
        if let Some(s) = chunk.store {
            let g = (s / 4) as usize;
            let rot = (s % 4 + self.shift) % 4;
            self.group_writes[g].push((dest, rot));
        }
        Some(dest)
    }

    /// Resolve an operand of statement 0 of a chunk to a vector value.
    fn resolve_arg(&mut self, arg: &Arg, entry: &[Value], _nslots: u8) -> Option<Value> {
        match arg {
            Arg::Slot { slot, version } => {
                let g = (*slot / 4) as usize;
                let read_lane = (slot % 4 + self.shift) % 4;
                if *version == 0 {
                    let src = entry.get(g).copied()?;
                    Some(self.rotated(src, 0, read_lane))
                } else {
                    let writes = &self.group_writes[g];
                    let w = writes.get((*version - 1) as usize)?;
                    let (val, rot) = *w;
                    Some(self.rotated(val, rot, read_lane))
                }
            }
            Arg::Op { index } => Some(self.op_result[*index]),
            Arg::Const(_) => None,
        }
    }
}

/// Latency (cycles) of a non-byte-multiple lane rotate as the x86 backend
/// lowers `VecRotlI32x4`: AVX-512VL `vprold` is one µop; otherwise the
/// shift pair runs in parallel and `vpor` joins it, 2 cycles.
pub(crate) fn arx_rot_latency() -> u32 {
    if crate::passes::vectorize::x86_avx512vl_available_pub() {
        1
    } else {
        2
    }
}

/// The ARX lane-vectorizers' latency model, shared by `vec_arx` and
/// `arx_vectorize` so both passes rank schedules by the same numbers.
/// Every op they emit is a 1-cycle vector-integer µop on Zen 3/4/5 and on
/// Intel P-cores (add, xor, pshufb, pshufd), except the rotate.
pub(crate) fn arx_vec_op_latency(op: IntrinsicOp, rot_latency: u32) -> u32 {
    match op {
        IntrinsicOp::VecRotlI32x4 => rot_latency,
        _ => 1,
    }
}

/// Critical-path length of a straight-line kernel under
/// `arx_vec_op_latency`, with every value defined outside it (loop phis,
/// preheader masks) ready at cycle 0.  For a round-loop body this is the
/// loop-carried recurrence per iteration: every result feeds the next
/// iteration's phis.
pub(crate) fn kernel_critical_path(insts: &[Instruction], rot_latency: u32) -> u32 {
    let mut ready: FxHashMap<u32, u32> = FxHashMap::default();
    let mut finish = 0;
    for inst in insts {
        if let Instruction::Intrinsic {
            dest: Some(d),
            op,
            args,
            ..
        } = inst
        {
            let start = args
                .iter()
                .filter_map(|a| match a {
                    Operand::Value(v) => ready.get(&v.0).copied(),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            let t = start + arx_vec_op_latency(*op, rot_latency);
            ready.insert(d.0, t);
            finish = finish.max(t);
        }
    }
    finish
}

/// Partition the chunks into op→op components: chunks linked by an
/// `Arg::Op` operand exchange values without passing through a slot, so
/// they must share one lane frame.  Returns a dense component id per chunk.
fn chunk_components(body: &Body, chunks: &[Chunk]) -> Vec<usize> {
    let mut op_chunk = vec![usize::MAX; body.ops.len()];
    for (ci, ch) in chunks.iter().enumerate() {
        for &m in &ch.op_indices {
            op_chunk[m] = ci;
        }
    }
    fn find(parent: &mut [usize], x: usize) -> usize {
        let mut root = x;
        while parent[root] != root {
            root = parent[root];
        }
        let mut y = x;
        while parent[y] != root {
            let next = parent[y];
            parent[y] = root;
            y = next;
        }
        root
    }
    let mut parent: Vec<usize> = (0..chunks.len()).collect();
    for (ci, ch) in chunks.iter().enumerate() {
        // Isomorphism makes every lane's Op operand land in the same chunk,
        // so statement 0 is representative.
        let op0 = &body.ops[ch.op_indices[0]];
        for arg in [&op0.a, &op0.b] {
            if let Arg::Op { index } = arg {
                let cj = op_chunk[*index];
                if cj != usize::MAX {
                    let (ra, rb) = (find(&mut parent, ci), find(&mut parent, cj));
                    parent[ra] = rb;
                }
            }
        }
    }
    let mut dense: FxHashMap<usize, usize> = FxHashMap::default();
    (0..chunks.len())
        .map(|ci| {
            let root = find(&mut parent, ci);
            let next = dense.len();
            *dense.entry(root).or_insert(next)
        })
        .collect()
}

/// Choose the lane shift of the component whose first chunk is `first`:
/// simulate each shift on a scratch emitter through the component's last
/// chunk (later, still-undecided components provisionally use shift 0) and
/// keep the one whose results are ready earliest; ties prefer fewer emitted
/// instructions, then the smaller shift (s = 0 is the historical frame).
#[allow(clippy::too_many_arguments)]
fn choose_lane_shift(
    base: &Emitter,
    body: &Body,
    chunks: &[Chunk],
    comp: &[usize],
    comp_shift: &[Option<u8>],
    first: usize,
    entry: &[Value],
    nslots: u8,
) -> Option<u8> {
    let c = comp[first];
    let last = (first..chunks.len()).rev().find(|&i| comp[i] == c)?;
    let mut best: Option<(u32, usize, u8)> = None;
    for s in 0..LANES as u8 {
        let mut e = base.clone();
        let mut finish = 0;
        for i in first..=last {
            e.shift = if comp[i] == c {
                s
            } else {
                comp_shift[comp[i]].unwrap_or(0)
            };
            let v = e.emit_chunk(body, &chunks[i], entry, nslots)?;
            if comp[i] == c {
                finish = finish.max(e.ready_of(v));
            }
        }
        let better = match best {
            None => true,
            Some((bf, bn, _)) => (finish, e.out.len()) < (bf, bn),
        };
        if better {
            best = Some((finish, e.out.len(), s));
        }
    }
    best.map(|(_, _, s)| s)
}

/// Public entry: transform one function. Returns the number of scalar ARX
/// ops vectorized (0 = no change).
pub fn vec_arx_function(func: &mut IrFunction) -> usize {
    // ISA gate: this entry runs in Phase 2b, before the main vectorizer's
    // `vectorize_with_analysis` gate, yet it emits the same XMM-shaped Vec*
    // intrinsics (VecLoadI32x4, VecShufdI32x4, ...).  A `-mno-sse` /
    // `-mgeneral-regs-only` TU (kernel: CR4.OSFXSR=0) must keep every loop
    // scalar — the x86 backend rightly rejects an XMM instruction there.
    // Mirrors the gate the const-trip map entry gained for the same reason.
    if !crate::passes::vectorize::x86_simd_available_pub() {
        return 0;
    }
    let cfg = CfgAnalysis::build(func);
    let dom = DominanceChecker::new(cfg.num_blocks, &cfg.idom);
    let loops =
        loop_analysis::find_merged_natural_loops(cfg.num_blocks, &cfg.preds, &cfg.succs, &cfg.idom);
    for lp in loops {
        if lp.len() > 2 {
            continue; // v1: 1- or 2-block loops
        }
        let (header, latch) = if lp.len() == 1 {
            (lp.header, lp.header) // self-loop: guard+body in one block
        } else {
            let latch = lp
                .body
                .iter()
                .copied()
                .find(|&b| b != lp.header)
                .unwrap_or(lp.header);
            (lp.header, latch)
        };
        match try_transform_loop(func, &dom, header, latch) {
            0 => continue,
            n => return n,
        }
    }
    0
}

fn try_transform_loop(
    func: &mut IrFunction,
    dom: &DominanceChecker,
    header: usize,
    latch: usize,
) -> usize {
    let header_label = func.blocks[header].label.0;
    let latch_label = func.blocks[latch].label.0;

    // Shape A (self-loop): the latch branches to itself.
    // Shape B (2-block): the latch ends in Branch(header); the header ends
    // in CondBranch(latch | exit).
    let self_loop = header == latch;
    if self_loop {
        match &func.blocks[latch].terminator {
            Terminator::CondBranch { true_label, .. } if true_label.0 == latch_label => {}
            _ => {
                return 0;
            }
        }
    } else {
        match &func.blocks[latch].terminator {
            Terminator::Branch(t) if t.0 == header_label => {}
            _ => {
                return 0;
            }
        }
        match &func.blocks[header].terminator {
            Terminator::CondBranch { true_label, .. } if true_label.0 == latch_label => {}
            _ => {
                return 0;
            }
        }
    }

    // Work block = the latch (it carries the ARX).
    let work = latch;

    // Discover the slot domain in the work block.
    let (base, nslots) = match discover_slot_domain(func, work) {
        Some(x) => x,
        None => {
            return 0;
        }
    };

    // Extract the flat op list (fail-closed).
    let body = match extract_body(func, work, base, nslots) {
        Some(b) => b,
        None => {
            return 0;
        }
    };

    // Escape check: no consumed value may be used by any instruction that
    // survives (outside the consumed set, in ANY block — including the
    // header's phis and the latch's own IV cluster), or by any terminator.
    {
        let consumed_vals: FxHashSet<u32> = body
            .consumed
            .iter()
            .filter_map(|&i| {
                let inst = &func.blocks[work].instructions[i];
                inst.dest().map(|v| v.0)
            })
            .collect();
        for (bi, b) in func.blocks.iter().enumerate() {
            for (ii, inst) in b.instructions.iter().enumerate() {
                if bi == work && body.consumed.contains(&ii) {
                    continue;
                }
                let mut esc = false;
                inst.for_each_used_value(|v| {
                    if consumed_vals.contains(&v) {
                        esc = true;
                    }
                });
                if esc {
                    return 0;
                }
            }
            let mut esc_term = false;
            b.terminator.for_each_used_value(|v| {
                if consumed_vals.contains(&v) {
                    esc_term = true;
                }
            });
            if esc_term {
                return 0;
            }
        }
    }

    // Chunk: discover K.
    let n_ops = body.ops.len();
    let mut chunks = None;
    for k in (4..=96).step_by(4) {
        if n_ops % (4 * k) != 0 {
            continue;
        }
        if let Some(c) = try_chunk(&body, k) {
            chunks = Some(c);
            break;
        }
    }
    let chunks = match chunks {
        Some(c) => c,
        None => return 0,
    };

    // Preheader: the single non-loop predecessor of the header.
    let loop_labels: FxHashSet<u32> = [header_label, latch_label].into_iter().collect();
    let mut preheaders: Vec<usize> = Vec::new();
    for (bi, b) in func.blocks.iter().enumerate() {
        if loop_labels.contains(&b.label.0) {
            continue;
        }
        if terminator_targets(&b.terminator)
            .iter()
            .any(|&t| t == header_label)
        {
            preheaders.push(bi);
        }
    }
    if preheaders.len() != 1 {
        return 0;
    }
    let preheader = preheaders[0];
    if !dom.dominates(preheader, header) {
        return 0;
    }

    // Exit edges: (source block, target block) pairs leaving the loop.
    let mut exits: Vec<(usize, u32)> = Vec::new();
    for (bi, b) in func.blocks.iter().enumerate() {
        if !loop_labels.contains(&b.label.0) {
            continue;
        }
        for &t in &terminator_targets(&b.terminator) {
            if !loop_labels.contains(&t) {
                exits.push((bi, t));
            }
        }
    }
    if exits.is_empty() {
        return 0;
    }
    // Soundness of exit re-materialization (SA-06).  The transform inserts
    // `VecStoreI32x4` of the packed state at the TOP of each exit block, so
    // those stores must execute EXACTLY ONCE per path reaching the exit.
    // Two CFG shapes break that invariant:
    //   (a) an exit block that is itself a loop header: the stores re-run on
    //       every iteration of that outer/feed-forward loop, and the
    //       synthesized phi's backedge incoming would be V_init (the PRE-
    //       round state), clobbering x[] mid-accumulation;
    //   (b) a predecessor that is not the loop or its preheader (an early
    //       `goto out` bypass): that path's own writes to x[] happen AFTER
    //       V_init was loaded in the preheader, so storing V_init back
    //       discards them, and V_init does not even dominate that edge.
    // Dominance of the preheader alone is NOT sufficient for either.
    // Fail closed: accept only exit targets whose predecessors are loop
    // blocks or the preheader itself, and which are not loop headers.
    // The ordinary ChaCha shape (one exit edge into straight-line code) is
    // unaffected; only these ambiguous CFGs decline.
    for &(src, target) in &exits {
        let _ = src;
        let exit_idx = match func.blocks.iter().position(|b| b.label.0 == target) {
            Some(x) => x,
            None => return 0,
        };
        for (bi, b) in func.blocks.iter().enumerate() {
            if !terminator_targets(&b.terminator).contains(&target) {
                continue;
            }
            // (a) exit dominates a predecessor ⇒ predecessor→exit is a
            //     backedge, so the exit block is a loop header.  Do NOT
            //     skip `bi == exit_idx`: a self-edge is a one-block loop
            //     whose stores would re-execute on every iteration, and
            //     `dominates` is reflexive, so it is caught here too.
            if dom.dominates(exit_idx, bi) {
                return 0;
            }
            if loop_labels.contains(&b.label.0) {
                continue;
            }
            // (b) only the preheader may supply V_init to an outside edge.
            if bi != preheader {
                return 0;
            }
        }
    }

    let groups = nslots as usize / LANES;
    let mut next_val = func.next_value_id.max(scan_max_value(func) + 1);

    // ── 1. Preheader pack: V_g from sixteen 4-byte loads. ──────────────
    // Built WITHOUT mutating the function: the chunk emission below can
    // still decline, and a bail after this point would leave orphaned
    // (dead) loads in the preheader — every decision is made before the
    // first mutation (the fail-closed discipline the map transform uses).
    //
    // The pack is ELEMENT-WISE on purpose (4 × `Load U32` +
    // `VecPackI32x4` per group, never a 16-byte `VecLoadI32x4`).  The
    // chacha20_block regression on Zen 5 (EPYC 9V45: 2.26x vs GCC at
    // -O2, while the same binary WON on Intel) was store-to-load
    // forwarding pathology around the state staging, and the control
    // experiment matters: GCC's own codegen performs the identical
    // "caller `movl` → `call` → 16-byte `movdqu` partial-overlap" read
    // of the input block and is fast on Zen 5, so that bounded stall
    // (~2 per call, both vendors) is NOT the cliff.  What only this
    // pass's old shape did was the staging ROUND-TRIP: a 16-byte vector
    // store to the state alloca immediately re-read by sixteen 4-byte
    // GPR loads for the pack.  Narrow GPR loads forwarded out of
    // vector-domain stores are the documented weak spot of the Zen
    // store queue (domain-crossing penalty / no-forward retry per
    // load), sixteen times per call, two million calls — invisible on
    // Intel, which forwards narrow-from-wide across domains cheaply.
    //
    // Element loads sidestep every variant of the problem: a 4-byte
    // load forwards exactly from a 4-byte store, as a contained load
    // from anything wider, and never crosses the vector/GPR domain
    // boundary, so the pack is on the fast path on BOTH vendors at the
    // cost of a handful of once-per-call pack µops that the ~460
    // dynamic instructions of the round loop dwarf.
    //
    // When the entry copy loop `x[i] = src[i]` is proven
    // (`match_entry_copy`), the pack reads `src` directly and the copy
    // loop is deleted at commit time: the staging store/reload round-trip
    // vanishes and the caller's freshly-stored words (the RFC 7539
    // counter update) are read with forwardable element loads.
    let entry_copy = match_entry_copy(func, dom, preheader, base, nslots);
    let gather_base = entry_copy.as_ref().map_or(base, |c| c.src);
    let mut v_init: Vec<Value> = Vec::with_capacity(groups);
    let mut entry_insts: Vec<Instruction> = Vec::with_capacity(groups * 9);
    for g in 0..groups {
        let mut lanes: Vec<Operand> = Vec::with_capacity(LANES);
        for l in 0..LANES {
            let p = Value(next_val);
            next_val += 1;
            entry_insts.push(Instruction::GetElementPtr {
                dest: p,
                base: gather_base,
                offset: Operand::Const(IrConst::I64((16 * g + 4 * l) as i64)),
                ty: IrType::Ptr,
            });
            let t = Value(next_val);
            next_val += 1;
            entry_insts.push(Instruction::Load {
                dest: t,
                ptr: p,
                ty: IrType::U32,
                seg_override: crate::common::types::AddressSpace::Default,
                volatile: false,
            });
            lanes.push(Operand::Value(t));
        }
        let d = Value(next_val);
        next_val += 1;
        v_init.push(d);
        entry_insts.push(Instruction::Intrinsic {
            dest: Some(d),
            op: IntrinsicOp::VecPackI32x4,
            dest_ptr: None,
            args: lanes,
        });
    }

    // ── 2. Loop-carried phis in the header: V_g = phi(init, latch). ─────
    let preheader_label = func.blocks[preheader].label.0;
    let mut v_phi: Vec<Value> = Vec::with_capacity(groups);
    let mut phi_insts: Vec<Instruction> = Vec::with_capacity(groups);
    for g in 0..groups {
        let d = Value(next_val);
        next_val += 1;
        v_phi.push(d);
        phi_insts.push(Instruction::Phi {
            dest: d,
            ty: IrType::U32, // lane type; result width comes from the op
            incoming: vec![
                (Operand::Value(v_init[g]), BlockId(preheader_label)),
                (Operand::Value(d), BlockId(latch_label)), // latch fixed below
            ],
        });
    }

    // ── 3. Emit the chunk bodies. ──────────────────────────────────────
    // ── 2b. Whole-byte rotate masks (SSSE3+): preheader-materialised,
    //        one per DISTINCT amount, loop-invariant.  Built pre-mutation,
    //        right after the entry loads they will follow into the
    //        preheader.  ChaCha's rot16/rot8 land here; rot12/rot7 keep
    //        the shift triple.
    // AVX-512VL: `vprold` is one µop for EVERY rotate amount, so the
    // byte-rotate mask materialisation is dead weight — decline it.
    let use_pshufb = crate::passes::vectorize::x86_sse41_available_pub()
        && !crate::passes::vectorize::x86_avx512vl_available_pub();
    let mut rot_masks: FxHashMap<i64, Value> = FxHashMap::default();
    if use_pshufb {
        let mut amounts: Vec<i64> = body
            .ops
            .iter()
            .filter(|op| op.op == IrBinOp::RotateLeft)
            .filter_map(|op| match &op.b {
                Arg::Const(c) => c.to_i64(),
                _ => None,
            })
            .filter(|&n| n % 8 == 0 && (1..=3).contains(&(n / 8)))
            .collect();
        amounts.sort_unstable();
        amounts.dedup();
        for n in amounts {
            let lanes = byterot_mask_dwords((n / 8) as u8);
            let d = Value(next_val);
            next_val += 1;
            entry_insts.push(Instruction::Intrinsic {
                dest: Some(d),
                op: IntrinsicOp::VecPackI32x4,
                dest_ptr: None,
                args: lanes
                    .iter()
                    .map(|&l| Operand::Const(IrConst::I32(l)))
                    .collect(),
            });
            rot_masks.insert(n, d);
        }
    }
    let mut emitter = Emitter {
        op_result: vec![Value(0); body.ops.len()],
        group_writes: vec![Vec::new(); groups],
        rot_cache: FxHashMap::default(),
        rot_masks,
        out: Vec::new(),
        next_val,
        shift: 0,
        ready: FxHashMap::default(),
        rot_latency: arx_rot_latency(),
    };
    let comp = chunk_components(&body, &chunks);
    let mut comp_shift: Vec<Option<u8>> = vec![None; comp.iter().max().map_or(0, |&c| c + 1)];
    for (ci, chunk) in chunks.iter().enumerate() {
        let shift = match comp_shift[comp[ci]] {
            Some(s) => s,
            None => {
                let Some(s) = choose_lane_shift(
                    &emitter,
                    &body,
                    &chunks,
                    &comp,
                    &comp_shift,
                    ci,
                    &v_phi,
                    nslots,
                ) else {
                    return 0;
                };
                comp_shift[comp[ci]] = Some(s);
                s
            }
        };
        emitter.shift = shift;
        if emitter.emit_chunk(&body, chunk, &v_phi, nslots).is_none() {
            return 0;
        }
    }

    // ── 4. Latch: canonical rotation 0 for every group. ────────────────
    let mut v_latch: Vec<Value> = Vec::with_capacity(groups);
    for g in 0..groups {
        let v = match emitter.group_writes[g].last() {
            Some(&(val, rot)) => emitter.rotated(val, rot, 0),
            None => v_phi[g], // never written: carries unchanged
        };
        v_latch.push(v);
    }
    for (g, phi) in phi_insts.iter_mut().enumerate() {
        if let Instruction::Phi { incoming, .. } = phi {
            incoming[1].0 = Operand::Value(v_latch[g]);
        }
    }
    let body_insts = std::mem::take(&mut emitter.out);
    next_val = emitter.next_val;

    // ── 4b. Commit the entry loads (all decline points are past). ─────
    func.blocks[preheader].extend_insts(entry_insts, Span::dummy());

    // Absorbed entry copy: the pack now reads `src` directly, so the
    // staging loop is dead.  Route the outside entry straight to the
    // preheader and gut the two copy blocks (their values are proven
    // loop-internal, so nothing can reference them).  The blocks stay in
    // place — unreachable and empty — for the CFG cleanup passes to drop;
    // removing Vec entries here would shift every later block index this
    // function still holds.
    if let Some(ec) = &entry_copy {
        let ph_label = func.blocks[preheader].label.0;
        let hdr_label = func.blocks[ec.copy_header].label.0;
        retarget_terminator(
            &mut func.blocks[ec.entry_pred].terminator,
            hdr_label,
            ph_label,
        );
        for &bi in &[ec.copy_header, ec.copy_body] {
            let blk = &mut func.blocks[bi];
            let _ = blk.take_parallel_debug();
            blk.instructions.clear();
            blk.terminator = Terminator::Branch(BlockId(ph_label));
        }
    }

    // ── 5. Rewrite the work (latch) block. ─────────────────────────────
    {
        let block = &mut func.blocks[work];
        // Surviving scalar instructions keep their spans; the vector body and
        // the phis are synthesized (dummy spans, no `.loc`).
        let (spans, ids) = block.take_parallel_debug();
        let tracked = !spans.is_empty();
        let mut kept: Vec<Instruction> = Vec::with_capacity(block.instructions.len());
        let mut kept_spans: Vec<Span> = Vec::with_capacity(spans.len());
        let mut kept_ids: Vec<u32> = Vec::with_capacity(ids.len());
        for (i, inst) in block.instructions.iter().enumerate() {
            if body.consumed.contains(&i) {
                continue;
            }
            kept.push(inst.clone());
            if tracked {
                kept_spans.push(spans[i]);
                kept_ids.push(
                    ids.get(i)
                        .copied()
                        .unwrap_or(crate::ir::instruction::NO_INLINE_INSTANCE),
                );
            }
        }
        let mut new_insts: Vec<Instruction> =
            Vec::with_capacity(kept.len() + body_insts.len() + phi_insts.len());
        let mut new_spans: Vec<Span> = Vec::new();
        let mut new_ids: Vec<u32> = Vec::new();
        let body_len = body_insts.len();
        let dummies = |n: usize| std::iter::repeat_n(Span::dummy(), n);
        if !self_loop {
            // 2-block shape: phis live in the header, not here.
            new_insts.extend(kept.iter().cloned());
            new_insts.extend(body_insts);
            if tracked {
                new_spans.extend(kept_spans.iter().copied());
                new_spans.extend(dummies(body_len));
                new_ids.extend(kept_ids.iter().copied());
                new_ids.extend(std::iter::repeat_n(
                    crate::ir::instruction::NO_INLINE_INSTANCE,
                    body_len,
                ));
            }
        } else {
            new_insts.append(&mut phi_insts.clone());
            let iv_start = kept.iter().position(|inst| {
                matches!(inst, Instruction::Copy { .. } | Instruction::Cmp { .. })
                    || matches!(
                        inst,
                        Instruction::BinOp {
                            ty: IrType::I32 | IrType::I64,
                            op: IrBinOp::Add,
                            ..
                        }
                    )
            });
            let iv_start = iv_start.unwrap_or(kept.len());
            new_insts.extend(kept.iter().take(iv_start).cloned());
            new_insts.extend(body_insts);
            new_insts.extend(kept.iter().skip(iv_start).cloned());
            if tracked {
                new_spans.extend(dummies(phi_insts.len()));
                new_spans.extend(kept_spans[..iv_start].iter().copied());
                new_spans.extend(dummies(body_len));
                new_spans.extend(kept_spans[iv_start..].iter().copied());
                new_ids.extend(std::iter::repeat_n(
                    crate::ir::instruction::NO_INLINE_INSTANCE,
                    phi_insts.len(),
                ));
                new_ids.extend(kept_ids[..iv_start].iter().copied());
                new_ids.extend(std::iter::repeat_n(
                    crate::ir::instruction::NO_INLINE_INSTANCE,
                    body_len,
                ));
                new_ids.extend(kept_ids[iv_start..].iter().copied());
            }
        }
        if ids.is_empty() {
            new_ids.clear();
        }
        block.replace_insts_inline(new_insts, new_spans, new_ids);
    }
    // Insert the header phis (2-block shape).
    if !self_loop {
        let hdr = &mut func.blocks[header];
        hdr.insert_insts(0, phi_insts, Span::dummy());
    }

    // ── 6. Exit materialization. ───────────────────────────────────────
    // Value on an exit edge: from the header (guard-style exit, state =
    // phi after the last completed backedge) → v_phi; from the latch
    // (body-style exit, state = this iteration's writes) → v_latch.
    //
    // When the entry copy was absorbed (so `v_init` holds the packed
    // ORIGINAL input) and the single exit falls into the canonical
    // finalization loop `out[i] = x[i] + src[i]`, the exit also absorbs
    // that loop: `out_g = V_exit_g + vecload(src)_g`, with the state
    // taken straight from vector registers, behind a runtime overlap
    // guard that keeps the scalar loop as the cold fallback.
    let exit_combine = match (&entry_copy, exits.as_slice()) {
        (Some(ec), [(_, target)]) => func
            .blocks
            .iter()
            .position(|b| b.label.0 == *target)
            .and_then(|exit_idx| {
                // Blocks whose `base` references are accounted for by the
                // transform itself (pack, rounds, gutted copy loop).  The
                // entry predecessor is deliberately NOT excluded: only the
                // Alloca definition may appear there.
                let mut round_blocks = vec![preheader, header, latch];
                round_blocks.extend([ec.copy_header, ec.copy_body]);
                match_exit_combine(func, dom, exit_idx, base, ec.src, nslots, &round_blocks)
            }),
        _ => None,
    };
    install_exit_materialization(
        func,
        &exits,
        base,
        &v_init,
        &v_phi,
        &v_latch,
        &mut next_val,
        nslots,
        dom,
        preheader,
        header,
        latch,
        exit_combine.as_ref(),
    );

    func.next_value_id = next_val.max(func.next_value_id);
    n_ops
}

/// Store the current state back to the array at every loop exit.
#[allow(clippy::too_many_arguments)]
fn install_exit_materialization(
    func: &mut IrFunction,
    exits: &[(usize, u32)],
    base: Value,
    v_init: &[Value],
    v_phi: &[Value],
    v_latch: &[Value],
    next_val: &mut u32,
    nslots: u8,
    dom: &DominanceChecker,
    preheader: usize,
    header: usize,
    latch: usize,
    combine: Option<&ExitCombine>,
) {
    let groups = nslots as usize / LANES;
    // Group exits by target block.
    let mut by_target: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for &(src, target) in exits {
        by_target.entry(target).or_default().push(src);
    }
    for (target, srcs) in by_target {
        let exit_idx = match func.blocks.iter().position(|b| b.label.0 == target) {
            Some(x) => x,
            None => continue,
        };
        // Predecessors of the exit block.
        let mut preds: Vec<usize> = Vec::new();
        for (bi, b) in func.blocks.iter().enumerate() {
            if bi == exit_idx {
                continue;
            }
            if terminator_targets(&b.terminator).contains(&target) {
                preds.push(bi);
            }
        }
        // The value per group on each incoming edge.
        let mut head: Vec<Instruction> = Vec::with_capacity(groups * 2);
        let mut stores: Vec<Instruction> = Vec::with_capacity(groups);
        let mut nv = *next_val;
        let mut store_vals: Vec<Value> = Vec::with_capacity(groups);
        for g in 0..groups {
            // Determine per-pred value; if all preds agree (or the single
            // pred is a loop block), no phi is needed.
            let val_for = |p: usize| -> Value {
                if p == header {
                    v_phi[g]
                } else if p == latch {
                    v_latch[g]
                } else {
                    v_init[g]
                }
            };
            let single = if preds.len() == 1 {
                Some(val_for(preds[0]))
            } else {
                let first = preds.first().copied().map(val_for);
                if first.is_some() && preds.iter().all(|&p| val_for(p) == first.unwrap()) {
                    first
                } else {
                    None
                }
            };
            let val = match single {
                Some(v) => v,
                None => {
                    // A phi is required (mixed edges). Note: header-exit
                    // and latch-exit values differ only when the guard
                    // runs before the body (2-block shape with an exit
                    // from BOTH blocks — rare; fail safe via phi).
                    let d = Value(nv);
                    nv += 1;
                    let mut inc: Vec<(Operand, BlockId)> = Vec::with_capacity(preds.len());
                    for &p in &preds {
                        inc.push((Operand::Value(val_for(p)), BlockId(func.blocks[p].label.0)));
                    }
                    head.push(Instruction::Phi {
                        dest: d,
                        ty: IrType::U32,
                        incoming: inc,
                    });
                    d
                }
            };
            store_vals.push(val);
        }
        for g in 0..groups {
            stores.push(Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreI32x4,
                dest_ptr: Some(base),
                args: vec![
                    Operand::Value(store_vals[g]),
                    Operand::Value(base),
                    Operand::Const(IrConst::I64(16 * g as i64)),
                ],
            });
        }
        *next_val = nv;
        let absorb = combine.filter(|c| c.exit_idx == exit_idx);
        let Some(c) = absorb else {
            let block = &mut func.blocks[exit_idx];
            block.insert_insts(0, head.into_iter().chain(stores), Span::dummy());
            let _ = dom;
            let _ = preheader;
            let _ = srcs;
            continue;
        };

        // ── Finalization-loop absorption ───────────────────────────────
        // exit_blk:  [head phis] (+ state spill stores if still needed)
        //            guard: scalar iff (out - src) - e  u<  (nslots-2)e+1
        //            CondBranch(overlap → scalar path, else → fast_blk)
        // spill_blk: [state spill stores] → scalar header   (only when
        //            the state is otherwise dead; keeps the hot path
        //            free of the dead stack traffic)
        // fast_blk:  out_g = VecAdd(V_exit_g, VecLoad(src, 16g));
        //            VecStore(out) → after
        //
        // The guard is one-sided ON PURPOSE: the packed `v_init` values
        // diverge from the scalar loop's reads only when `out` sits
        // 1..nslots-1 elements above `src` (see `match_exit_combine`).
        // Distances below one element are misaligned-lvalue UB on the
        // typed streams, the same element-granular contract
        // `emit_alias_guards` documents.
        let src = c.src;
        let elem = 4i64;
        let span = (i64::from(nslots) - 2) * elem + 1;
        let hdr_label = func.blocks[c.ep_hdr].label.0;
        let exit_label = func.blocks[exit_idx].label.0;
        // `next_label == 0` is the documented "not yet computed" state;
        // allocating from it would collide with existing labels.  Seed
        // it from the actual label population first.
        let max_label = func.blocks.iter().map(|b| b.label.0 + 1).max().unwrap_or(0);
        func.next_label = func.next_label.max(max_label);
        let fast_label = BlockId(func.next_label);
        func.next_label += 1;
        let spill_label = if c.state_dead_after {
            let l = BlockId(func.next_label);
            func.next_label += 1;
            Some(l)
        } else {
            None
        };

        let mut nv = *next_val;
        let mut fresh = || {
            let v = Value(nv);
            nv += 1;
            v
        };
        let d_out = fresh();
        let d_src = fresh();
        let dist = fresh();
        let dist_e = fresh();
        let overlap = fresh();
        let guard = [
            Instruction::Cast {
                dest: d_out,
                src: Operand::Value(c.out_base),
                from_ty: IrType::Ptr,
                to_ty: IrType::I64,
            },
            Instruction::Cast {
                dest: d_src,
                src: Operand::Value(src),
                from_ty: IrType::Ptr,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: dist,
                op: IrBinOp::Sub,
                lhs: Operand::Value(d_out),
                rhs: Operand::Value(d_src),
                ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: dist_e,
                op: IrBinOp::Sub,
                lhs: Operand::Value(dist),
                rhs: Operand::Const(IrConst::I64(elem)),
                ty: IrType::I64,
            },
            Instruction::Cmp {
                dest: overlap,
                op: IrCmpOp::Ult,
                lhs: Operand::Value(dist_e),
                rhs: Operand::Const(IrConst::I64(span)),
                ty: IrType::I64,
            },
        ];

        // The feed-forward RE-LOADS `src` with full 16-byte vector loads
        // instead of reusing the preheader pack's `v_init` registers.
        // Reusing them reads nicer but keeps four extra XMM registers
        // live across the whole round loop, and at x86-64-v3 (where the
        // two pshufb rotate masks already occupy registers) that
        // overflows the register file and puts a spill/reload pair
        // INSIDE the hot loop — strictly worse than any exit cost.  The
        // wide loads are safe from the store-forwarding cliff that
        // motivated the element-wise ENTRY pack: by exit time the
        // caller's narrow stores retired hundreds of cycles ago (the
        // round loop alone is ~400+ cycles), so these are plain L1 hits
        // overlapping nothing in the store queue.  All fast-path loads
        // execute before any `out` store, which is exactly the ordering
        // the overlap guard's one-sided check was proven against.
        let mut fast: Vec<Instruction> = Vec::with_capacity(groups * 3);
        for g in 0..groups {
            let w = fresh();
            fast.push(Instruction::Intrinsic {
                dest: Some(w),
                op: IntrinsicOp::VecLoadI32x4,
                dest_ptr: None,
                args: vec![
                    Operand::Value(src),
                    Operand::Const(IrConst::I64(16 * g as i64)),
                ],
            });
            let d = fresh();
            fast.push(Instruction::Intrinsic {
                dest: Some(d),
                op: IntrinsicOp::VecAddI32x4,
                dest_ptr: None,
                args: vec![Operand::Value(store_vals[g]), Operand::Value(w)],
            });
            fast.push(Instruction::Intrinsic {
                dest: None,
                op: IntrinsicOp::VecStoreI32x4,
                dest_ptr: Some(c.out_base),
                args: vec![
                    Operand::Value(d),
                    Operand::Value(c.out_base),
                    Operand::Const(IrConst::I64(16 * g as i64)),
                ],
            });
        }
        *next_val = nv;

        let scalar_target = spill_label.unwrap_or(BlockId(hdr_label));
        {
            let block = &mut func.blocks[exit_idx];
            let spilled_here: Vec<Instruction> = if spill_label.is_some() {
                Vec::new()
            } else {
                stores.clone()
            };
            block.insert_insts(0, head.into_iter().chain(spilled_here), Span::dummy());
            block.extend_insts(guard, Span::dummy());
            block.terminator = Terminator::CondBranch {
                cond: Operand::Value(overlap),
                true_label: scalar_target,
                false_label: fast_label,
            };
        }
        if let Some(sl) = spill_label {
            // The scalar fallback still reads the state array, so the
            // spill stores live on its (cold) path.  Its iv phi's
            // incoming edge moves from the exit block to the new one.
            func.blocks.push(BasicBlock {
                label: sl,
                instructions: Vec::new(),
                terminator: Terminator::Branch(BlockId(hdr_label)),
                source_spans: vec![],
                source_inline_ids: Vec::new(),
            });
            let si = func.blocks.len() - 1;
            func.blocks[si].extend_insts(stores, Span::dummy());
            for inst in func.blocks[c.ep_hdr].instructions.iter_mut() {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (_, lbl) in incoming.iter_mut() {
                        if lbl.0 == exit_label {
                            *lbl = sl;
                        }
                    }
                }
            }
        }
        func.blocks.push(BasicBlock {
            label: fast_label,
            instructions: Vec::new(),
            terminator: Terminator::Branch(BlockId(c.after_label)),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        let fi = func.blocks.len() - 1;
        func.blocks[fi].extend_insts(fast, Span::dummy());
    }
}

/// The canonical rolled finalization loop `for (i = 0; i < nslots; i++)
/// out[i] = x[i] + src[i];` following the matched round loop, resolved
/// to its CFG pieces so the exit can absorb it (see
/// `match_exit_combine`).
struct ExitCombine {
    /// Block index of the round loop's exit block (sole predecessor of
    /// the finalization loop from outside).
    exit_idx: usize,
    /// Block index of the finalization loop's header (phi + bound cmp).
    ep_hdr: usize,
    /// Block index of the finalization loop's body.
    ep_body: usize,
    /// Label of the block control reaches after the finalization loop.
    after_label: u32,
    /// Destination base pointer (`out`) — ParamRef/Alloca/GlobalAddr
    /// dominating the exit block.
    out_base: Value,
    /// The original input pointer whose packed values `v_init` holds
    /// (the matched entry copy's `src`); the runtime overlap guard
    /// compares `out` against it.
    src: Value,
    /// Whether the state array has no readers besides the finalization
    /// loop, so the exit stores of the packed state can move onto the
    /// (cold) scalar-fallback path only.
    state_dead_after: bool,
}

/// Recognize the finalization loop `out[i] = x[i] + src[i]` that
/// follows a ChaCha-style round loop, so the exit can compute
/// `out_g = V_final_g + vecload(src)_g`: the permuted state comes
/// straight from vector registers and the input via wide vector loads
/// (no reloads of `x`, no scalar walk of `src`).
///
/// `src` is unchanged since function entry — the round loop was proven
/// to write only the state alloca — so the exit-time loads see exactly
/// the values the scalar loop would read.  The caller gates this on a
/// successful `match_entry_copy` of the same `src` (the canonical
/// paired shape; it also guarantees `src`'s provenance was vetted).
///
/// Aliasing between `out` and `src` is handled by a RUNTIME guard the
/// absorber emits (see `install_exit_materialization`): the fast path
/// issues ALL its `src` loads before ANY `out` store, which diverges
/// from the scalar loop's interleaved reads/writes only when `out`
/// sits 1..nslots-1 elements ABOVE `src` (iteration i's store to
/// `out[i]` lands on a not-yet-read `src[j]`, j > i).  `out == src`
/// exactly, `out` below `src`, and disjoint buffers are all
/// read-before-clobber under either order.  The guard is the element-granular
/// one-sided check the general vectorizer's `emit_alias_guards` uses:
/// scalar iff `(out - src) - e` is unsigned-below `(nslots-2)*e + 1`.
///
/// `out` vs the state array needs no guard: the matcher requires the
/// state base to be a non-escaping `Alloca`, which no pointer the
/// caller supplied (ParamRef/GlobalAddr) and no distinct alloca can
/// address.
fn match_exit_combine(
    func: &IrFunction,
    dom: &DominanceChecker,
    exit_idx: usize,
    base: Value,
    src: Value,
    nslots: u8,
    round_blocks: &[usize],
) -> Option<ExitCombine> {
    // Exit block falls through to the loop header unconditionally.
    let Terminator::Branch(BlockId(hdr_label)) = func.blocks[exit_idx].terminator else {
        return None;
    };
    let exit_label = func.blocks[exit_idx].label.0;
    let p = func.blocks.iter().position(|b| b.label.0 == hdr_label)?;
    if p == exit_idx {
        return None;
    }

    // Header: exactly [Phi(I32), Cmp(Slt, iv, nslots)] + CondBranch.
    let hdr = &func.blocks[p];
    if hdr.instructions.len() != 2 {
        return None;
    }
    let Instruction::Phi {
        dest: iv,
        ty: IrType::I32,
        incoming,
    } = &hdr.instructions[0]
    else {
        return None;
    };
    let iv = *iv;
    let Instruction::Cmp {
        dest: cmp_dest,
        op: IrCmpOp::Slt,
        lhs: Operand::Value(cmp_lhs),
        rhs: Operand::Const(IrConst::I32(bound)),
        ty: IrType::I32,
    } = hdr.instructions[1]
    else {
        return None;
    };
    if cmp_lhs != iv || bound != i32::from(nslots) {
        return None;
    }
    let Terminator::CondBranch {
        cond: Operand::Value(cond),
        true_label: BlockId(body_label),
        false_label: BlockId(after_label),
    } = hdr.terminator
    else {
        return None;
    };
    if cond != cmp_dest || body_label == hdr_label || after_label == hdr_label {
        return None;
    }
    let b_idx = func.blocks.iter().position(|b| b.label.0 == body_label)?;
    if b_idx == p || b_idx == exit_idx || body_label == after_label {
        return None;
    }
    // The after-block must exist and carry no phis (the absorber adds a
    // new incoming edge to it; fail closed rather than patch phis).
    let after_idx = func.blocks.iter().position(|b| b.label.0 == after_label)?;
    if after_idx == p || after_idx == b_idx || after_idx == exit_idx {
        return None;
    }
    if func.blocks[after_idx]
        .instructions
        .iter()
        .any(|i| matches!(i, Instruction::Phi { .. }))
    {
        return None;
    }

    // Phi incoming: exactly {0 from the exit block, step from the body}.
    if incoming.len() != 2 {
        return None;
    }
    let mut init_ok = false;
    let mut step_val: Option<Value> = None;
    for (op, BlockId(from)) in incoming {
        match op {
            Operand::Const(IrConst::I32(0)) if *from == exit_label => init_ok = true,
            Operand::Value(v) if *from == body_label => step_val = Some(*v),
            _ => return None,
        }
    }
    let step = step_val?;
    if !init_ok {
        return None;
    }

    // Body: exactly the ten instructions of `out[i] = x[i] + src[i];
    // i++`, wired together, ending in a branch back to the header.
    let body = &func.blocks[b_idx];
    if !matches!(body.terminator, Terminator::Branch(BlockId(l)) if l == hdr_label) {
        return None;
    }
    if body.instructions.len() != 10 {
        return None;
    }
    let mut cast_dest: Option<Value> = None;
    let mut off_dest: Option<Value> = None;
    let mut geps: Vec<(Value, Value)> = Vec::new(); // (dest, base)
    let mut loads: Vec<(Value, Value)> = Vec::new(); // (dest, ptr)
    let mut sum: Option<(Value, Value, Value)> = None; // (dest, lhs, rhs)
    let mut store: Option<(Value, Value)> = None; // (val, ptr)
    let mut step_ok = false;
    for inst in &body.instructions {
        match inst {
            Instruction::Cast {
                dest,
                src: Operand::Value(s),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            } if *s == iv && cast_dest.is_none() => cast_dest = Some(*dest),
            Instruction::BinOp {
                dest,
                op: IrBinOp::Shl,
                lhs: Operand::Value(l),
                rhs: Operand::Const(IrConst::I64(2)),
                ty: IrType::I64,
            } if Some(*l) == cast_dest && off_dest.is_none() => off_dest = Some(*dest),
            Instruction::GetElementPtr {
                dest,
                base: gb,
                offset: Operand::Value(o),
                ..
            } if Some(*o) == off_dest => geps.push((*dest, *gb)),
            Instruction::Load {
                dest,
                ptr,
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            } => loads.push((*dest, *ptr)),
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs: Operand::Value(l),
                rhs: Operand::Value(r),
                ty: IrType::U32,
            } if sum.is_none() => sum = Some((*dest, *l, *r)),
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs: Operand::Value(l),
                rhs: Operand::Const(IrConst::I32(1)),
                ty: IrType::I32,
            } if *l == iv && *dest == step => step_ok = true,
            Instruction::Store {
                val: Operand::Value(v),
                ptr,
                ty: IrType::U32,
                seg_override: AddressSpace::Default,
                volatile: false,
            } if store.is_none() => store = Some((*v, *ptr)),
            _ => return None,
        }
    }
    if !step_ok || geps.len() != 3 || loads.len() != 2 {
        return None;
    }
    let (sum_dest, sum_l, sum_r) = sum?;
    let (store_val, store_ptr) = store?;
    if store_val != sum_dest {
        return None;
    }
    // Resolve the three GEPs: one over `base`, one over `src`, and the
    // store target over a third pointer `out`.
    let gep_of = |b: Value| -> Option<Value> {
        let mut found = None;
        for &(d, gb) in &geps {
            if gb == b {
                if found.is_some() {
                    return None; // ambiguous
                }
                found = Some(d);
            }
        }
        found
    };
    let g_x = gep_of(base)?;
    let g_s = gep_of(src)?;
    let (out_base, g_out) = geps
        .iter()
        .find(|&&(d, gb)| d != g_x && d != g_s && gb != base && gb != src)
        .map(|&(d, gb)| (gb, d))?;
    if store_ptr != g_out {
        return None;
    }
    // The two loads read x[i] and src[i]; the sum adds exactly them.
    let load_from =
        |ptr: Value| -> Option<Value> { loads.iter().find(|&&(_, p)| p == ptr).map(|&(d, _)| d) };
    let xv = load_from(g_x)?;
    let sv = load_from(g_s)?;
    if xv == sv || !((sum_l == xv && sum_r == sv) || (sum_l == sv && sum_r == xv)) {
        return None;
    }

    // Sole entries: header from {exit, body} only; body from header only.
    for (bi, blk) in func.blocks.iter().enumerate() {
        let targets = terminator_targets(&blk.terminator);
        if targets.contains(&hdr_label) && bi != exit_idx && bi != b_idx {
            return None;
        }
        if targets.contains(&body_label) && bi != p {
            return None;
        }
    }

    // `out` provenance: a pointer that dominates the exit block and
    // provably cannot address the state alloca.  `out == src` is fine
    // (distance 0 takes the fast path, which was proven exact).
    if out_base == base {
        return None;
    }
    let mut out_ok = false;
    let mut base_is_alloca = false;
    'defs: for (bi, blk) in func.blocks.iter().enumerate() {
        for inst in &blk.instructions {
            match inst {
                Instruction::ParamRef { dest, .. }
                | Instruction::Alloca { dest, .. }
                | Instruction::GlobalAddr { dest, .. }
                    if *dest == out_base =>
                {
                    out_ok = dom.dominates(bi, exit_idx);
                    if base_is_alloca {
                        break 'defs;
                    }
                }
                Instruction::Alloca { dest, .. } if *dest == base => {
                    base_is_alloca = true;
                    if out_ok {
                        break 'defs;
                    }
                }
                _ => {}
            }
        }
    }
    if !out_ok || !base_is_alloca {
        return None;
    }
    // The state alloca must not escape: its only permitted uses are as
    // a GEP base, a scalar load/store ADDRESS, or the pointer operand
    // of the pass's own vector load/store intrinsics.  Any other
    // appearance (stored as a value, passed to a call, phi/copy/cast
    // input, ...) could give `out` a path to alias it.  The presence
    // test is the CANONICAL `for_each_used_value` traversal (every
    // operand position, including pointers and call arguments); the
    // whitelist then re-derives which positions carried it, so a use
    // in a position the whitelist does not name can never slip by.
    for blk in &func.blocks {
        for inst in &blk.instructions {
            let mut uses_base = false;
            inst.for_each_used_value(|v| {
                if v == base.0 {
                    uses_base = true;
                }
            });
            if !uses_base {
                continue;
            }
            let ok = match inst {
                Instruction::GetElementPtr {
                    base: gb, offset, ..
                } => *gb == base && !matches!(offset, Operand::Value(v) if *v == base),
                Instruction::Load { ptr, .. } => *ptr == base,
                Instruction::Store { ptr, val, .. } => {
                    *ptr == base && !matches!(val, Operand::Value(v) if *v == base)
                }
                Instruction::Intrinsic { op, args, .. } => match op {
                    // args = [ptr, offset]: only the pointer lane may
                    // carry the alloca.
                    IntrinsicOp::VecLoadI32x4 => {
                        matches!(args.first(), Some(Operand::Value(v)) if *v == base)
                            && !args[1..]
                                .iter()
                                .any(|a| matches!(a, Operand::Value(v) if *v == base))
                    }
                    // args = [val, ptr, offset]: only the pointer lane.
                    IntrinsicOp::VecStoreI32x4 => {
                        matches!(args.get(1), Some(Operand::Value(v)) if *v == base)
                            && !matches!(args.first(), Some(Operand::Value(v)) if *v == base)
                            && !args[2..]
                                .iter()
                                .any(|a| matches!(a, Operand::Value(v) if *v == base))
                    }
                    _ => false,
                },
                _ => false,
            };
            if !ok {
                return None;
            }
        }
        let mut used_in_term = false;
        blk.terminator.for_each_used_value(|v| {
            if v == base.0 {
                used_in_term = true;
            }
        });
        if used_in_term {
            return None;
        }
    }

    // No finalization-loop value may be used outside it.
    let mut defined: FxHashSet<Value> = FxHashSet::default();
    defined.extend([
        iv, cmp_dest, step, sum_dest, store_val, g_x, g_s, g_out, xv, sv,
    ]);
    if let Some(c) = cast_dest {
        defined.insert(c);
    }
    if let Some(o) = off_dest {
        defined.insert(o);
    }
    for (bi, blk) in func.blocks.iter().enumerate() {
        if bi == p || bi == b_idx {
            continue;
        }
        let mut leaked = false;
        for inst in &blk.instructions {
            inst.for_each_used_value(|v| {
                if defined.contains(&Value(v)) {
                    leaked = true;
                }
            });
        }
        blk.terminator.for_each_used_value(|v| {
            if defined.contains(&Value(v)) {
                leaked = true;
            }
        });
        if leaked {
            return None;
        }
    }

    // Does anything OTHER than the finalization loop read the state
    // array after the rounds?  If not, the packed-state spill stores
    // only need to exist on the scalar-fallback path.
    let mut known: FxHashSet<usize> = round_blocks.iter().copied().collect();
    known.extend([exit_idx, p, b_idx]);
    let mut state_dead_after = true;
    for (bi, blk) in func.blocks.iter().enumerate() {
        if known.contains(&bi) {
            continue;
        }
        for inst in &blk.instructions {
            // The canonical traversal covers every use position (GEP
            // bases, load/store pointers, call arguments, casts); the
            // Alloca definition itself reports no uses.
            inst.for_each_used_value(|v| {
                if v == base.0 {
                    state_dead_after = false;
                }
            });
        }
    }

    Some(ExitCombine {
        exit_idx,
        ep_hdr: p,
        ep_body: b_idx,
        after_label,
        out_base,
        src,
        state_dead_after,
    })
}

/// The canonical rolled entry-copy loop `for (i = 0; i < nslots; i++)
/// x[i] = src[i];` feeding the matched state array, resolved to its CFG
/// pieces so the pack can absorb it (see `match_entry_copy`).
struct EntryCopy {
    /// Loop-invariant source base pointer (a `ParamRef`, a distinct
    /// `Alloca`, or a `GlobalAddr` — the three shapes that provably
    /// cannot alias the state alloca).
    src: Value,
    /// Block index of the copy loop's header (phi + bound compare).
    copy_header: usize,
    /// Block index of the copy loop's body (the load/store pair).
    copy_body: usize,
    /// Block index whose terminator enters the copy loop from outside;
    /// its edge is re-targeted straight to the round preheader.
    entry_pred: usize,
}

/// Recognize the entry-copy loop so the preheader pack can read the
/// ORIGINAL source array element-wise instead of staging through the
/// state alloca.
///
/// # Why this transform exists (the Zen store-forwarding cliff)
///
/// The measured 2.26x chacha20_block regression vs GCC on EPYC 9V45
/// (Zen 5) — while the identical binary beat GCC on Intel hosts — came
/// from the staging shape this matcher removes.  The copy loop was
/// vectorized by the general vectorizer into 16-byte `movdqu` loads of
/// `src` and 16-byte vector STORES into the state alloca, which the
/// preheader pack then immediately re-read with sixteen 4-byte GPR
/// loads.  Forwarding a narrow integer load out of a vector-domain
/// store is the Zen store queue's documented weak spot (domain-crossing
/// penalty per load), ×16 per call, ×2M calls; Intel forwards
/// narrow-from-wide across domains cheaply, which is why the Intel
/// numbers never showed it.  The control experiment that pins this
/// down: GCC's winning Zen 5 code performs the very same caller-`movl`
/// → `call` → partial-overlap `movdqu` reads of the input block, so
/// that bounded (~2 per call) stall is demonstrably not the cliff —
/// the round-trip through the alloca is.
///
/// Absorbing the copy loop deletes the round-trip entirely: the pack
/// issues four 4-byte loads per group (`VecPackI32x4` of four scalar
/// `Load U32`) straight from `src`.  A 4-byte load forwards exactly
/// from the caller's 4-byte store, as a contained load from anything
/// wider, and never crosses the vector/GPR domain boundary — the fast
/// path on every Intel and AMD core.  The copy loop's 4+4 vector
/// load/store pairs plus the pack's 16 reloads collapse into 16 loads
/// with no intermediate stores, shortening the per-call critical path
/// on hazard-free microarchitectures too.
///
/// # Fail-closed matching rules (every one is load-bearing)
///
/// * The preheader must be instruction-free: anything there executes
///   between the copy and the pack and could redefine either array.
/// * The preheader's ONLY predecessor is the copy-loop header; the
///   header is exactly `[Phi(i: 0 | step), Cmp(i <s nslots)]` +
///   `CondBranch(body | preheader)`; the body is exactly the seven
///   instructions of `x[i] = src[i]; i++` and branches back.  Any extra
///   instruction, a volatile access, or a non-default address space
///   rejects (the transform then keeps the state-array pack, which is
///   correct for every shape).
/// * `i` runs 0..nslots by 1 (`Slt` against exactly `nslots`), so every
///   slot the round loop reads is covered — a partial copy must keep
///   staging through the array.
/// * `src` must be a `ParamRef`, a different `Alloca`, or a
///   `GlobalAddr`, defined in a block dominating the copy header.  Each
///   of the three provably cannot alias the state alloca (a fresh
///   non-escaped alloca is disjoint from caller memory, from every other
///   alloca, and from globals), so the copy's loads read memory the
///   copy's own stores never touch, and moving those loads to the
///   preheader — past the deleted stores — reads identical bytes.  A
///   computed pointer (loaded from memory, GEP result, call result)
///   could point INTO the state array and is rejected.
/// * The copy loop has no other entries (header preds = {entry, body},
///   body preds = {header}) and none of its values escape — deleting it
///   cannot orphan a use.
fn match_entry_copy(
    func: &IrFunction,
    dom: &DominanceChecker,
    preheader: usize,
    base: Value,
    nslots: u8,
) -> Option<EntryCopy> {
    if !func.blocks[preheader].instructions.is_empty() {
        return None;
    }
    let ph_label = func.blocks[preheader].label.0;

    // Sole predecessor of the preheader = candidate copy header.
    let mut ph_preds = func
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| terminator_targets(&b.terminator).contains(&ph_label))
        .map(|(bi, _)| bi);
    let p = ph_preds.next()?;
    if ph_preds.next().is_some() || p == preheader {
        return None;
    }
    let p_label = func.blocks[p].label.0;

    // Header: CondBranch { body | preheader } over exactly [Phi, Cmp].
    let hdr = &func.blocks[p];
    let (hdr_cond, body_label) = match &hdr.terminator {
        Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } if false_label.0 == ph_label && true_label.0 != p_label && true_label.0 != ph_label => {
            (cond, true_label.0)
        }
        _ => return None,
    };
    if hdr.instructions.len() != 2 {
        return None;
    }
    let Instruction::Phi {
        dest: iv,
        ty: IrType::I32,
        incoming,
    } = &hdr.instructions[0]
    else {
        return None;
    };
    let Instruction::Cmp {
        dest: cmp_dest,
        op: crate::ir::ops::IrCmpOp::Slt,
        lhs: Operand::Value(cmp_lhs),
        rhs: Operand::Const(cmp_rhs),
        ty: IrType::I32,
    } = &hdr.instructions[1]
    else {
        return None;
    };
    if cmp_lhs != iv
        || cmp_rhs.to_i64() != Some(i64::from(nslots))
        || *hdr_cond != Operand::Value(*cmp_dest)
    {
        return None;
    }
    // Phi: (Const 0 from the outside entry, step from the body).
    let [a, b] = &incoming[..] else { return None };
    let ((init, entry_label), (step_op, step_label)) = if a.1.0 == body_label {
        ((&b.0, b.1.0), (&a.0, a.1.0))
    } else {
        ((&a.0, a.1.0), (&b.0, b.1.0))
    };
    if step_label != body_label || entry_label == body_label {
        return None;
    }
    match init {
        Operand::Const(c) if c.to_i64() == Some(0) => {}
        _ => return None,
    }
    let Operand::Value(step) = step_op else {
        return None;
    };

    // Body: exactly the seven instructions of `x[i] = src[i]; i++`.
    let b_idx = func.blocks.iter().position(|b| b.label.0 == body_label)?;
    let body = &func.blocks[b_idx];
    match &body.terminator {
        Terminator::Branch(t) if t.0 == p_label => {}
        _ => return None,
    }
    if body.instructions.len() != 7 {
        return None;
    }
    fn set_once<T>(slot: &mut Option<T>, v: T) -> Option<()> {
        if slot.is_some() {
            return None; // duplicate role: not the canonical shape
        }
        *slot = Some(v);
        Some(())
    }
    let mut cast: Option<Value> = None; // i widened to I64
    let mut shl: Option<(Value, Value)> = None; // (dest, lhs)
    let mut gep_x: Option<(Value, Value)> = None; // (dest, offset)
    let mut gep_s: Option<(Value, Value, Value)> = None; // (dest, base, offset)
    let mut load: Option<(Value, Value)> = None; // (dest, ptr)
    let mut store: Option<(Value, Value)> = None; // (val, ptr)
    let mut add: Option<Value> = None; // i + 1
    for inst in &body.instructions {
        match inst {
            Instruction::Cast {
                dest,
                src: Operand::Value(s),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            } if s == iv => set_once(&mut cast, *dest)?,
            Instruction::BinOp {
                dest,
                op: IrBinOp::Shl,
                lhs: Operand::Value(l),
                rhs: Operand::Const(c),
                ty: IrType::I64,
            } if c.to_i64() == Some(2) => set_once(&mut shl, (*dest, *l))?,
            Instruction::GetElementPtr {
                dest,
                base: gb,
                offset: Operand::Value(o),
                ..
            } => {
                if *gb == base {
                    set_once(&mut gep_x, (*dest, *o))?
                } else {
                    set_once(&mut gep_s, (*dest, *gb, *o))?
                }
            }
            Instruction::Load {
                dest,
                ptr,
                ty: IrType::U32,
                seg_override: crate::common::types::AddressSpace::Default,
                volatile: false,
            } => set_once(&mut load, (*dest, *ptr))?,
            Instruction::Store {
                val: Operand::Value(v),
                ptr,
                ty: IrType::U32,
                seg_override: crate::common::types::AddressSpace::Default,
                volatile: false,
            } => set_once(&mut store, (*v, *ptr))?,
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs,
                rhs,
                ty: IrType::I32,
            } => {
                let one = |o: &Operand| matches!(o, Operand::Const(c) if c.to_i64() == Some(1));
                let is_iv = |o: &Operand| matches!(o, Operand::Value(v) if v == iv);
                if (is_iv(lhs) && one(rhs)) || (is_iv(rhs) && one(lhs)) {
                    set_once(&mut add, *dest)?
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let (cast, (shl_d, shl_l)) = (cast?, shl?);
    let ((gx_d, gx_o), (gs_d, src, gs_o)) = (gep_x?, gep_s?);
    let ((ld_d, ld_p), (st_v, st_p), add_d) = (load?, store?, add?);
    // Wire the roles together; any mismatch is a different loop.
    if shl_l != cast
        || gx_o != shl_d
        || gs_o != shl_d
        || ld_p != gs_d
        || st_v != ld_d
        || st_p != gx_d
        || add_d != *step
    {
        return None;
    }

    // Outside entry block: targets the header, is none of the loop blocks.
    let entry_pred = func.blocks.iter().position(|b| b.label.0 == entry_label)?;
    if entry_pred == p
        || entry_pred == b_idx
        || entry_pred == preheader
        || !terminator_targets(&func.blocks[entry_pred].terminator).contains(&p_label)
    {
        return None;
    }

    // No other entries into the loop.
    for (bi, blk) in func.blocks.iter().enumerate() {
        let targets = terminator_targets(&blk.terminator);
        if targets.contains(&p_label) && bi != entry_pred && bi != b_idx {
            return None;
        }
        if targets.contains(&body_label) && bi != p {
            return None;
        }
    }

    // `src` must provably not alias the state alloca: its defining
    // instruction (the canonical `dest()` accessor sees every
    // def-bearing variant) must be a ParamRef, a distinct Alloca, or a
    // GlobalAddr dominating the copy loop.
    let mut src_ok = false;
    'outer: for (bi, blk) in func.blocks.iter().enumerate() {
        for inst in &blk.instructions {
            if inst.dest() == Some(src) {
                src_ok = matches!(
                    inst,
                    Instruction::ParamRef { .. }
                        | Instruction::Alloca { .. }
                        | Instruction::GlobalAddr { .. }
                ) && dom.dominates(bi, p);
                break 'outer;
            }
        }
    }
    if !src_ok {
        return None;
    }

    // No copy-loop value may be used outside the loop (canonical
    // traversal: every operand position, including pointers).
    let copy_vals: FxHashSet<u32> = [
        iv.0, cmp_dest.0, cast.0, shl_d.0, gx_d.0, gs_d.0, ld_d.0, add_d.0,
    ]
    .into_iter()
    .collect();
    for (bi, blk) in func.blocks.iter().enumerate() {
        if bi == p || bi == b_idx {
            continue;
        }
        let mut esc = false;
        for inst in &blk.instructions {
            inst.for_each_used_value(|v| {
                if copy_vals.contains(&v) {
                    esc = true;
                }
            });
        }
        blk.terminator.for_each_used_value(|v| {
            if copy_vals.contains(&v) {
                esc = true;
            }
        });
        if esc {
            return None;
        }
    }

    Some(EntryCopy {
        src,
        copy_header: p,
        copy_body: b_idx,
        entry_pred,
    })
}

/// Replace every occurrence of label `from` with `to` in a terminator.
fn retarget_terminator(t: &mut Terminator, from: u32, to: u32) {
    let fix = |b: &mut BlockId| {
        if b.0 == from {
            b.0 = to;
        }
    };
    match t {
        Terminator::Branch(b) => fix(b),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            fix(true_label);
            fix(false_label);
        }
        Terminator::Switch { cases, default, .. } => {
            for (_, b) in cases.iter_mut() {
                fix(b);
            }
            fix(default);
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for b in possible_targets.iter_mut() {
                fix(b);
            }
        }
        Terminator::Return(_) | Terminator::Unreachable => {}
    }
}

fn terminator_targets(t: &Terminator) -> Vec<u32> {
    // Exhaustive on purpose (no wildcard arm): a new CFG-bearing variant
    // must be handled here or the crate stops compiling.  A wildcard would
    // silently hide a computed-goto edge from the exit-CFG proof below.
    match t {
        Terminator::Branch(b) => vec![b.0],
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => vec![true_label.0, false_label.0],
        Terminator::Switch { cases, default, .. } => {
            let mut v: Vec<u32> = cases.iter().map(|(_, x)| x.0).collect();
            v.push(default.0);
            v
        }
        // SA-06 red-team: `goto *ptr` (computed goto) can reach every label
        // registered in `possible_targets`.  Those ARE real CFG edges; the
        // old `_ => Vec::new()` made them invisible to the exit-predecessor
        // proof, so a bypass through an indirect branch could be
        // clobbered by the inserted VecStoreI32x4.  List them explicitly.
        Terminator::IndirectBranch {
            possible_targets, ..
        } => possible_targets.iter().map(|b| b.0).collect(),
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
    }
}

fn scan_max_value(func: &IrFunction) -> u32 {
    // Canonical def/use traversals: the old narrow local walkers missed
    // Cast destinations and pointer-position uses, so a function whose
    // numerically largest value id sat in one of those positions could
    // hand out a COLLIDING id for the pass's fresh values.
    let mut max = 0u32;
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Some(v) = inst.dest() {
                max = max.max(v.0);
            }
            inst.for_each_used_value(|v| max = max.max(v));
        }
        b.terminator.for_each_used_value(|v| max = max.max(v));
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pshufd_immediates_are_the_classic_sse2_rotations() {
        // dest lane l ← src lane (l+1)%4 is "rotate lanes left by one",
        // 0x39 — the classic _MM_SHUFFLE(0,3,2,1) of SSE2 ChaCha20.
        assert_eq!(pshufd_imm(1), 0x39);
        assert_eq!(pshufd_imm(2), 0x4E);
        assert_eq!(pshufd_imm(3), 0x93);
        assert_eq!(pshufd_imm(0), 0xE4);
        assert_eq!(pshufd_imm(4), 0xE4);
    }

    #[test]
    fn kernel_critical_path_is_the_longest_latency_chain() {
        let v = |n: u32| Operand::Value(Value(n));
        let op = |dest: u32, op: IntrinsicOp, args: Vec<Operand>| Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op,
            dest_ptr: None,
            args,
        };
        // Values 1 and 2 are loop-entry state (ready at 0).
        // 10 = 1 + 2; 11 = rotl(10, 7); 12 = pshufd(11); 13 = 12 ^ 1
        // (the long chain), plus a short side chain 14 = 1 ^ 2.
        let kernel = vec![
            op(10, IntrinsicOp::VecAddI32x4, vec![v(1), v(2)]),
            op(
                11,
                IntrinsicOp::VecRotlI32x4,
                vec![v(10), Operand::Const(IrConst::I32(7))],
            ),
            op(
                12,
                IntrinsicOp::VecShufdI32x4,
                vec![v(11), Operand::Const(IrConst::I32(0x39))],
            ),
            op(13, IntrinsicOp::VecXorI32x4, vec![v(12), v(1)]),
            op(14, IntrinsicOp::VecXorI32x4, vec![v(1), v(2)]),
        ];
        // Shift-pair rotate (2 cycles): 1 + 2 + 1 + 1.
        assert_eq!(kernel_critical_path(&kernel, 2), 5);
        // vprold (1 cycle): 1 + 1 + 1 + 1.
        assert_eq!(kernel_critical_path(&kernel, 1), 4);
        assert_eq!(kernel_critical_path(&[], 2), 0);
    }

    #[test]
    fn pshufd_rotation_roundtrips() {
        for r in 1u8..4 {
            let fwd = pshufd_imm(r);
            let bwd = pshufd_imm(4 - r);
            let sel = |imm: i32, l: u8| -> u8 { ((imm >> (2 * l)) & 0b11) as u8 };
            for l in 0u8..4 {
                let mid = sel(fwd, l);
                assert_eq!(sel(bwd, mid), l, "roundtrip r={}", r);
            }
        }
    }

    /// SA-06 red-team: every real CFG edge must be visible to the exit
    /// predecessor proof.  A wildcard `_ => Vec::new()` used to hide
    /// computed-goto targets, so an indirect bypass into the exit block
    /// could be clobbered by the inserted state stores.
    #[test]
    fn terminator_targets_cover_every_cfg_edge() {
        let b = |n: u32| BlockId(n);
        assert_eq!(terminator_targets(&Terminator::Branch(b(7))), vec![7]);
        assert_eq!(
            terminator_targets(&Terminator::CondBranch {
                cond: Operand::Const(IrConst::I32(0)),
                true_label: b(1),
                false_label: b(2),
            }),
            vec![1, 2]
        );
        assert_eq!(
            terminator_targets(&Terminator::Switch {
                val: Operand::Const(IrConst::I32(0)),
                cases: vec![(0, b(3)), (1, b(4))],
                default: b(5),
                ty: IrType::I32,
            }),
            vec![3, 4, 5]
        );
        assert_eq!(
            terminator_targets(&Terminator::IndirectBranch {
                target: Operand::Value(Value(9)),
                possible_targets: vec![b(6), b(7), b(8)],
            }),
            vec![6, 7, 8]
        );
        assert!(terminator_targets(&Terminator::Return(None)).is_empty());
        assert!(terminator_targets(&Terminator::Unreachable).is_empty());
    }

    // ───────────────────── boundary-absorption matchers ─────────────────
    //
    // Fixture: the canonical ChaCha20 block-function CFG exactly as the
    // frontend lowers it (labels == indices):
    //
    //   b0 entry (alloca x, params out/in)      → b1
    //   b1 copy header  (i phi, i <s 16)        → b2 | b3
    //   b2 copy body    (x[i] = in[i]; i++)     → b1
    //   b3 round preheader (empty)              → b4
    //   b4 round loop   (self-loop stand-in)    → b4 | b5
    //   b5 round exit   (empty)                 → b6
    //   b6 final header (j phi, j <s 16)        → b7 | b8
    //   b7 final body   (out[j] = x[j] + in[j]) → b6
    //   b8 after                                → ret
    const X: Value = Value(4); // state alloca
    const OUT: Value = Value(2); // param 0
    const IN: Value = Value(3); // param 1

    fn u32_load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::U32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    fn u32_store(val: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty: IrType::U32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    fn iv_block(phi: u32, step: u32, cmp: u32, bound: i32, from: (u32, u32)) -> Vec<Instruction> {
        vec![
            Instruction::Phi {
                dest: Value(phi),
                ty: IrType::I32,
                incoming: vec![
                    (Operand::Const(IrConst::I32(0)), BlockId(from.0)),
                    (Operand::Value(Value(step)), BlockId(from.1)),
                ],
            },
            Instruction::Cmp {
                dest: Value(cmp),
                op: IrCmpOp::Slt,
                lhs: Operand::Value(Value(phi)),
                rhs: Operand::Const(IrConst::I32(bound)),
                ty: IrType::I32,
            },
        ]
    }
    fn addr_insts(iv: u32, cast: u32, shl: u32) -> Vec<Instruction> {
        vec![
            Instruction::Cast {
                dest: Value(cast),
                src: Operand::Value(Value(iv)),
                from_ty: IrType::I32,
                to_ty: IrType::I64,
            },
            Instruction::BinOp {
                dest: Value(shl),
                op: IrBinOp::Shl,
                lhs: Operand::Value(Value(cast)),
                rhs: Operand::Const(IrConst::I64(2)),
                ty: IrType::I64,
            },
        ]
    }
    fn gep(dest: u32, base: Value, off: u32) -> Instruction {
        Instruction::GetElementPtr {
            dest: Value(dest),
            base,
            offset: Operand::Value(Value(off)),
            ty: IrType::Ptr,
        }
    }
    fn iv_step(dest: u32, iv: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(iv)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        }
    }
    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
            source_inline_ids: Vec::new(),
        }
    }
    fn cond(cmp: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(cmp)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }
    fn chacha_fixture() -> IrFunction {
        let mut f = IrFunction::new("chacha_core".to_string(), IrType::Void, vec![], false);
        let b0 = block(
            0,
            vec![
                Instruction::Alloca {
                    dest: X,
                    ty: IrType::Ptr,
                    size: 64,
                    align: 4,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::ParamRef {
                    dest: OUT,
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: IN,
                    param_idx: 1,
                    ty: IrType::Ptr,
                },
            ],
            Terminator::Branch(BlockId(1)),
        );
        let b1 = block(1, iv_block(100, 105, 101, 16, (0, 2)), cond(101, 2, 3));
        let mut copy_body = addr_insts(100, 102, 103);
        copy_body.push(gep(104, X, 103));
        copy_body.push(gep(106, IN, 103));
        copy_body.push(u32_load(107, 106));
        copy_body.push(u32_store(107, 104));
        copy_body.push(iv_step(105, 100));
        let b2 = block(2, copy_body, Terminator::Branch(BlockId(1)));
        let b3 = block(3, vec![], Terminator::Branch(BlockId(4)));
        let mut round = iv_block(110, 111, 112, 10, (3, 4));
        round.insert(1, iv_step(111, 110));
        let b4 = block(4, round, cond(112, 4, 5));
        let b5 = block(5, vec![], Terminator::Branch(BlockId(6)));
        let b6 = block(6, iv_block(120, 121, 122, 16, (5, 7)), cond(122, 7, 8));
        let mut fin_body = addr_insts(120, 123, 124);
        fin_body.push(gep(125, OUT, 124));
        fin_body.push(gep(126, X, 124));
        fin_body.push(u32_load(127, 126));
        fin_body.push(gep(128, IN, 124));
        fin_body.push(u32_load(129, 128));
        fin_body.push(Instruction::BinOp {
            dest: Value(130),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(127)),
            rhs: Operand::Value(Value(129)),
            ty: IrType::U32,
        });
        fin_body.push(u32_store(130, 125));
        fin_body.push(iv_step(121, 120));
        let b7 = block(7, fin_body, Terminator::Branch(BlockId(6)));
        let b8 = block(8, vec![], Terminator::Return(None));
        f.blocks = vec![b0, b1, b2, b3, b4, b5, b6, b7, b8];
        f.next_value_id = 200;
        f.next_label = 9;
        f
    }
    fn dom_of(f: &IrFunction) -> DominanceChecker {
        let cfg = CfgAnalysis::build(f);
        DominanceChecker::new(cfg.num_blocks, &cfg.idom)
    }
    const ROUND_BLOCKS: [usize; 4] = [3, 4, 1, 2];

    #[test]
    fn entry_copy_matches_the_canonical_chacha_shape() {
        let f = chacha_fixture();
        let dom = dom_of(&f);
        let ec = match_entry_copy(&f, &dom, 3, X, 16).expect("canonical shape must match");
        assert_eq!(ec.src, IN);
        assert_eq!(ec.copy_header, 1);
        assert_eq!(ec.copy_body, 2);
        assert_eq!(ec.entry_pred, 0);
    }

    #[test]
    fn entry_copy_declines_on_any_perturbation() {
        // Volatile load: the copy is observable, absorbing it would
        // delete a side effect.
        let mut f = chacha_fixture();
        if let Instruction::Load { volatile, .. } = &mut f.blocks[2].instructions[4] {
            *volatile = true;
        }
        assert!(match_entry_copy(&f, &dom_of(&f), 3, X, 16).is_none());

        // Bound mismatch: a partial copy must not seed the full pack.
        let mut f = chacha_fixture();
        if let Instruction::Cmp { rhs, .. } = &mut f.blocks[1].instructions[1] {
            *rhs = Operand::Const(IrConst::I32(12));
        }
        assert!(match_entry_copy(&f, &dom_of(&f), 3, X, 16).is_none());

        // Copied value escapes the loop: deleting the loop would orphan
        // the use.
        let mut f = chacha_fixture();
        f.blocks[8].instructions.push(Instruction::Copy {
            dest: Value(140),
            src: Operand::Value(Value(107)),
        });
        assert!(match_entry_copy(&f, &dom_of(&f), 3, X, 16).is_none());

        // Store aimed at the SOURCE array (reversed copy direction).
        let mut f = chacha_fixture();
        f.blocks[2].instructions[2] = gep(104, IN, 103);
        f.blocks[2].instructions[3] = gep(106, X, 103);
        assert!(match_entry_copy(&f, &dom_of(&f), 3, X, 16).is_none());

        // Second predecessor of the copy header (goto back into the
        // loop): iteration count is no longer provable.
        let mut f = chacha_fixture();
        f.blocks[8].terminator = Terminator::Branch(BlockId(1));
        assert!(match_entry_copy(&f, &dom_of(&f), 3, X, 16).is_none());

        // Non-empty preheader: code would execute between copy and pack.
        let mut f = chacha_fixture();
        f.blocks[3].instructions.push(iv_step(141, 100));
        assert!(match_entry_copy(&f, &dom_of(&f), 3, X, 16).is_none());
    }

    #[test]
    fn exit_combine_matches_and_proves_state_death() {
        let f = chacha_fixture();
        let dom = dom_of(&f);
        let c = match_exit_combine(&f, &dom, 5, X, IN, 16, &ROUND_BLOCKS)
            .expect("canonical feed-forward must match");
        assert_eq!(c.exit_idx, 5);
        assert_eq!(c.ep_hdr, 6);
        assert_eq!(c.ep_body, 7);
        assert_eq!(c.after_label, 8);
        assert_eq!(c.out_base, OUT);
        assert_eq!(c.src, IN);
        assert!(
            c.state_dead_after,
            "nothing outside the matched loops reads x[]"
        );
    }

    #[test]
    fn exit_combine_keeps_spill_stores_when_state_is_read_later() {
        let mut f = chacha_fixture();
        // The after-block inspects x[0]: the packed state must still be
        // spilled on every path.
        f.blocks[8].instructions.push(Instruction::Cast {
            dest: Value(151),
            src: Operand::Const(IrConst::I32(0)),
            from_ty: IrType::I32,
            to_ty: IrType::I64,
        });
        f.blocks[8].instructions.push(gep(150, X, 151));
        f.blocks[8].instructions.push(u32_load(152, 150));
        let dom = dom_of(&f);
        let c = match_exit_combine(&f, &dom, 5, X, IN, 16, &ROUND_BLOCKS)
            .expect("extra reader only demotes the spill, not the match");
        assert!(!c.state_dead_after);
    }

    #[test]
    fn exit_combine_declines_on_any_perturbation() {
        // A phi in the after-block: the absorber adds an incoming edge
        // it would have to patch.
        let mut f = chacha_fixture();
        f.blocks[8].instructions.push(Instruction::Phi {
            dest: Value(160),
            ty: IrType::I32,
            incoming: vec![(Operand::Const(IrConst::I32(0)), BlockId(6))],
        });
        assert!(match_exit_combine(&f, &dom_of(&f), 5, X, IN, 16, &ROUND_BLOCKS).is_none());

        // An 11th instruction in the body: not the canonical loop.
        let mut f = chacha_fixture();
        f.blocks[7].instructions.push(iv_step(161, 120));
        assert!(match_exit_combine(&f, &dom_of(&f), 5, X, IN, 16, &ROUND_BLOCKS).is_none());

        // The destination is the state array itself.
        let mut f = chacha_fixture();
        f.blocks[7].instructions[2] = gep(125, X, 124);
        assert!(match_exit_combine(&f, &dom_of(&f), 5, X, IN, 16, &ROUND_BLOCKS).is_none());

        // The state alloca escapes (stored as a value): `out` could
        // alias it through the escaped copy.
        let mut f = chacha_fixture();
        f.blocks[0].instructions.push(Instruction::Store {
            val: Operand::Value(X),
            ptr: OUT,
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        assert!(match_exit_combine(&f, &dom_of(&f), 5, X, IN, 16, &ROUND_BLOCKS).is_none());

        // A finalization value leaks out of the loop.
        let mut f = chacha_fixture();
        f.blocks[8].instructions.push(Instruction::Copy {
            dest: Value(162),
            src: Operand::Value(Value(130)),
        });
        assert!(match_exit_combine(&f, &dom_of(&f), 5, X, IN, 16, &ROUND_BLOCKS).is_none());

        // The sum reads something other than x[j] + in[j].
        let mut f = chacha_fixture();
        if let Instruction::BinOp { lhs, .. } = &mut f.blocks[7].instructions[7] {
            *lhs = Operand::Value(Value(129));
        }
        assert!(match_exit_combine(&f, &dom_of(&f), 5, X, IN, 16, &ROUND_BLOCKS).is_none());
    }

    #[test]
    fn retarget_terminator_rewrites_each_edge_kind() {
        let mut t = Terminator::Branch(BlockId(1));
        retarget_terminator(&mut t, 1, 9);
        assert_eq!(terminator_targets(&t), vec![9]);

        let mut t = cond(0, 1, 2);
        retarget_terminator(&mut t, 2, 9);
        assert_eq!(terminator_targets(&t), vec![1, 9]);

        let mut t = Terminator::Switch {
            val: Operand::Const(IrConst::I32(0)),
            cases: vec![(0, BlockId(1)), (1, BlockId(2))],
            default: BlockId(1),
            ty: IrType::I32,
        };
        retarget_terminator(&mut t, 1, 9);
        assert_eq!(terminator_targets(&t), vec![9, 2, 9]);
    }
}
