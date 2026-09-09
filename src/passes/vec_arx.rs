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
//! * `rol(d,n)` → `pslld + psrld + por` (the SSE2 rotate triple)
//!
//! The diagonal round reads its b/c/d operands from *lane-rotated* frames
//! (`QR_l` uses `x[4h + (l+r_h)%4]`); those frames materialize with one
//! `pshufd` each and rotate back on the next round's reads — the classic
//! SSE2 single-block ChaCha20 shuffle trick (6 `pshufd` per double round),
//! derived here mechanically from the slot pattern instead of hard-coded.
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
use crate::common::types::IrType;
use crate::ir::analysis::CfgAnalysis;
use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::ops::IrBinOp;
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

/// Emission context.
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
        dest
    }

    /// Resolve an operand of statement 0 of a chunk to a vector value.
    fn resolve_arg(&mut self, arg: &Arg, entry: &[Value], _nslots: u8) -> Option<Value> {
        match arg {
            Arg::Slot { slot, version } => {
                let g = (*slot / 4) as usize;
                let read_lane = slot % 4;
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

/// Public entry: transform one function. Returns the number of scalar ARX
/// ops vectorized (0 = no change).
pub fn vec_arx_function(func: &mut IrFunction) -> usize {
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
                inst_dest_value(inst).map(|v| v.0)
            })
            .collect();
        for (bi, b) in func.blocks.iter().enumerate() {
            for (ii, inst) in b.instructions.iter().enumerate() {
                if bi == work && body.consumed.contains(&ii) {
                    continue;
                }
                let mut esc = false;
                for_each_operand_in(inst, &mut |op| {
                    if let Operand::Value(v) = op {
                        if consumed_vals.contains(&v.0) {
                            esc = true;
                        }
                    }
                });
                if esc {
                    return 0;
                }
            }
            let mut esc_term = false;
            for_each_operand_in_terminator(&b.terminator, &mut |op| {
                if let Operand::Value(v) = op {
                    if consumed_vals.contains(&v.0) {
                        esc_term = true;
                    }
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
    let mut k_used = 0usize;
    for k in (4..=96).step_by(4) {
        if n_ops % (4 * k) != 0 {
            continue;
        }
        if let Some(c) = try_chunk(&body, k) {
            chunks = Some(c);
            k_used = k;
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
    // Every exit target's OTHER predecessors must be dominated by the
    // preheader (V_init live on those edges).
    for &(src, target) in &exits {
        let _ = src;
        let exit_idx = match func.blocks.iter().position(|b| b.label.0 == target) {
            Some(x) => x,
            None => return 0,
        };
        for (bi, b) in func.blocks.iter().enumerate() {
            if bi == exit_idx || loop_labels.contains(&b.label.0) {
                continue;
            }
            if terminator_targets(&b.terminator).contains(&target) && !dom.dominates(preheader, bi)
            {
                return 0;
            }
        }
    }

    let groups = nslots as usize / LANES;
    let mut next_val = func.next_value_id.max(scan_max_value(func) + 1);

    // ── 1. Preheader: V_g = VecLoadI32x4(base, 16g). ────────────────────
    // Built WITHOUT mutating the function: the chunk emission below can
    // still decline, and a bail after this point would leave orphaned
    // (dead) loads in the preheader — every decision is made before the
    // first mutation (the fail-closed discipline the map transform uses).
    let mut v_init: Vec<Value> = Vec::with_capacity(groups);
    let mut entry_insts: Vec<Instruction> = Vec::with_capacity(groups);
    for g in 0..groups {
        let d = Value(next_val);
        next_val += 1;
        v_init.push(d);
        entry_insts.push(Instruction::Intrinsic {
            dest: Some(d),
            op: IntrinsicOp::VecLoadI32x4,
            dest_ptr: None,
            args: vec![
                Operand::Value(base),
                Operand::Const(IrConst::I64(16 * g as i64)),
            ],
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
    let use_pshufb = crate::passes::vectorize::x86_sse41_available_pub();
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
    };
    for chunk in &chunks {
        let op0 = &body.ops[chunk.op_indices[0]];
        let is_rot = op0.op == IrBinOp::RotateLeft;
        let a = match emitter.resolve_arg(&op0.a, &v_phi, nslots) {
            Some(v) => v,
            None => return 0,
        };
        // A rotate's `b` is the constant AMOUNT (extract_body enforces
        // 1..=31), never a vector operand; `resolve_arg` refuses Const
        // args by design, so it must not be consulted for it.
        let b = if is_rot {
            None
        } else {
            match emitter.resolve_arg(&op0.b, &v_phi, nslots) {
                Some(v) => Some(v),
                None => return 0,
            }
        };
        let dest = emitter.fresh();
        // Whole-byte rotate with a materialised mask: one vpshufb
        // (args = [data, mask]) instead of the shift triple.
        let shufb_mask = if is_rot {
            match &op0.b {
                Arg::Const(c) => match c.to_i64() {
                    Some(n) => emitter.rot_masks.get(&n).copied(),
                    None => return 0,
                },
                _ => return 0,
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
            _ => return 0,
        };
        let mut args = vec![Operand::Value(a)];
        if let Some(mask_v) = shufb_mask {
            args.push(Operand::Value(mask_v));
        } else if is_rot {
            if let Arg::Const(c) = &op0.b {
                args.push(Operand::Const(c.clone()));
            } else {
                return 0;
            }
        } else {
            args.push(Operand::Value(b.expect("non-rotate has vector b")));
        }
        emitter.out.push(Instruction::Intrinsic {
            dest: Some(dest),
            op: intrinsic_op,
            dest_ptr: None,
            args,
        });
        for &m in &chunk.op_indices {
            emitter.op_result[m] = dest;
        }
        if let Some(s) = chunk.store {
            let g = (s / 4) as usize;
            let rot = s % 4;
            emitter.group_writes[g].push((dest, rot));
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
    func.blocks[preheader].instructions.extend(entry_insts);

    // ── 5. Rewrite the work (latch) block. ─────────────────────────────
    {
        let block = &mut func.blocks[work];
        let mut kept: Vec<Instruction> = Vec::with_capacity(block.instructions.len());
        for (i, inst) in block.instructions.iter().enumerate() {
            if body.consumed.contains(&i) {
                continue;
            }
            kept.push(inst.clone());
        }
        let mut new_insts: Vec<Instruction> =
            Vec::with_capacity(kept.len() + body_insts.len() + phi_insts.len());
        if !self_loop {
            // 2-block shape: phis live in the header, not here.
            new_insts.extend(kept.iter().cloned());
            new_insts.extend(body_insts);
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
        }
        block.instructions = new_insts;
    }
    // Insert the header phis (2-block shape).
    if !self_loop {
        let hdr = &mut func.blocks[header];
        let mut new_insts: Vec<Instruction> =
            Vec::with_capacity(hdr.instructions.len() + phi_insts.len());
        new_insts.extend(phi_insts);
        new_insts.extend(hdr.instructions.iter().cloned());
        hdr.instructions = new_insts;
    }

    // ── 6. Exit materialization. ───────────────────────────────────────
    // Value on an exit edge: from the header (guard-style exit, state =
    // phi after the last completed backedge) → v_phi; from the latch
    // (body-style exit, state = this iteration's writes) → v_latch.
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
        let block = &mut func.blocks[exit_idx];
        let mut new_insts: Vec<Instruction> =
            Vec::with_capacity(block.instructions.len() + head.len() + stores.len());
        new_insts.extend(head);
        new_insts.extend(stores);
        new_insts.extend(block.instructions.iter().cloned());
        block.instructions = new_insts;
        let _ = dom;
        let _ = preheader;
        let _ = srcs;
    }
}

fn terminator_targets(t: &Terminator) -> Vec<u32> {
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
        _ => Vec::new(),
    }
}

fn inst_dest_value(inst: &Instruction) -> Option<Value> {
    use Instruction::*;
    match inst {
        BinOp { dest, .. }
        | Load { dest, .. }
        | Phi { dest, .. }
        | GetElementPtr { dest, .. }
        | Copy { dest, .. }
        | Cmp { dest, .. } => Some(*dest),
        Intrinsic { dest, .. } => *dest,
        _ => None,
    }
}

fn for_each_operand_in(inst: &Instruction, f: &mut dyn FnMut(&Operand)) {
    use Instruction::*;
    match inst {
        BinOp { lhs, rhs, .. } | Cmp { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        Store { val, .. } => f(val),
        Phi { incoming, .. } => {
            for (o, _) in incoming {
                f(o);
            }
        }
        Intrinsic { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Copy { src, .. } => f(src),
        _ => {}
    }
}

fn for_each_operand_in_terminator(t: &Terminator, f: &mut dyn FnMut(&Operand)) {
    if let Terminator::CondBranch { cond, .. } = t {
        f(cond);
    }
}

fn scan_max_value(func: &IrFunction) -> u32 {
    let mut max = 0u32;
    for b in &func.blocks {
        for inst in &b.instructions {
            if let Some(v) = inst_dest_value(inst) {
                max = max.max(v.0);
            }
            for_each_operand_in(inst, &mut |op| {
                if let Operand::Value(v) = op {
                    max = max.max(v.0);
                }
            });
        }
        for_each_operand_in_terminator(&b.terminator, &mut |op| {
            if let Operand::Value(v) = op {
                max = max.max(v.0);
            }
        });
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
}
