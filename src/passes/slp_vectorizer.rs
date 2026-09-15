//! Basic-block SLP (Superword-Level Parallelism) vectorizer.
//!
//! Packs isomorphic scalar operations inside a single basic block into the
//! `Vec*` intrinsic families, seeded from stores to consecutive addresses.
//! This is the classic bottom-up SLP construction (Larsen & Amarasinghe
//! style) specialized for LCCC's vector-intrinsic IR:
//!
//! 1. **Seeds** — runs of stores to the same symbolic address stream with
//!    constant byte offsets consecutive at exactly the lane stride
//!    (`p->a..d = ...`, unrolled `a[i..i+3] = ...`, 4×u64 struct copies).
//! 2. **Pack graph** — from each seed the stored values are packed
//!    bottom-up: same-op `BinOp` lanes recurse into operand packs,
//!    consecutive-address `Load` lanes become vector loads, an all-same
//!    operand becomes a broadcast, and unbuildable sides degrade to
//!    2-/4-lane gathers where the family has one.
//! 3. **Legality** — four rules, all mandatory (see `build_plan`):
//!    (a) external uses of a packed lane must be scheduled after the
//!    vector op (else the replacing extract would precede its def);
//!    (b) a packed lane may not be used outside this block (the extract
//!    lives here; cross-block SSA repair needs dominator proof — v2);
//!    (c) no memory write may sit strictly between the lanes of a
//!    vectorized load (it could change which lanes see the old vs. new
//!    value); (d) nothing may access memory strictly between the seed
//!    stores (the vector store commits all lanes at once, so an
//!    interleaved reader could observe a different half-stored state).
//! 4. **Cost model** — benefit = Σ(replaced scalar ops − 1 per pack) −
//!    gather/extract costs; the seed fires only at benefit ≥ 1.
//!
//! FP correctness needs **no reassociation**: lane-parallel vector ops
//! compute exactly what each scalar lane computed, bit-for-bit (same
//! per-lane instruction semantics, same operand order for the
//! non-commutative Sub/Div). Integer Add/Sub/Mul/And/Or/Xor wrap in two's
//! complement exactly like the scalar IR ops (signed overflow is UB in C;
//! `unsigned` wraps by definition; `-fwrapv` addition also wraps — paddq/
//! psubq/pmullw wrap identically). This pass therefore runs under strict
//! FP semantics as well as fast-math.
//!
//! x86-64 only (the `Vec*` families have no other-backend lowerings),
//! -O2+ (not -Os/-Oz), kill switches `CCC_NO_BB_SLP` /
//! `CCC_DISABLE_PASSES=slp`, debug `LCCC_DEBUG_SLP=1`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::constants::IrConst;
use crate::ir::instruction::{BasicBlock, Instruction, Operand, Terminator, Value};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::ops::IrBinOp;
use crate::ir::reexports::IrFunction;
use crate::passes::vectorize::{x86_avx2_available_pub, x86_simd_available_pub};

/// Hashable lane encoding for pack dedup. Exact — no lossy constant
/// folding, so distinct constants can never collide into one pack.
/// LongDouble stores its f64 approximation bit-exactly alongside the 16
/// binary128 bytes (f64 has no Hash — keep the raw bits only).
#[derive(Clone, PartialEq, Eq, Hash)]
enum LaneKey {
    V(u32),
    I(i128),
    F32(u32),
    F64(u64),
    LD(u64, [u8; 16]),
    D32(u32),
    D64(u64),
}

fn lane_key(op: &Operand) -> LaneKey {
    match op {
        Operand::Value(v) => LaneKey::V(v.0),
        Operand::Const(c) => match c {
            IrConst::I8(v) => LaneKey::I(*v as i128),
            IrConst::I16(v) => LaneKey::I(*v as i128),
            IrConst::I32(v) => LaneKey::I(*v as i128),
            IrConst::I64(v) => LaneKey::I(*v as i128),
            IrConst::I128(v) => LaneKey::I(*v),
            IrConst::Zero => LaneKey::I(0),
            IrConst::F32(v) => LaneKey::F32(v.to_bits()),
            IrConst::F64(v) => LaneKey::F64(v.to_bits()),
            IrConst::LongDouble(f, b) => LaneKey::LD(f.to_bits(), *b),
            IrConst::D32(v) => LaneKey::D32(*v),
            IrConst::D64(v) => LaneKey::D64(*v),
        },
    }
}

/// Maximum packs in one seed's tree (diamond-heavy blocks bail out instead
/// of exploding).
const MAX_PACKS: usize = 64;

// ─────────────────────────────────────────────────────────────────────────
// Symbolic affine addresses
// ─────────────────────────────────────────────────────────────────────────

/// `base + var*mult + off` (bytes). `var` is an SSA index value shared by
/// every access of one stream; `mult` its byte scale.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct SymAddr {
    base: Value,
    var: Option<Value>,
    mult: u64,
    off: i64,
}

/// Evaluate a pointer value to a symbolic address. Recognizes raw pointer
/// values, `GEP(base, Const)` chains, and `GEP(base, Add(var, Const))`
/// (the partially-unrolled `a[i+k]` shape). Everything else is opaque.
fn eval_sym_addr(
    block: &BasicBlock,
    def_pos: &FxHashMap<u32, usize>,
    ptr: Value,
) -> Option<SymAddr> {
    let inst = block.instructions.get(*def_pos.get(&ptr.0)?)?;
    match inst {
        Instruction::GetElementPtr { base, offset, .. } => {
            // Base of a GEP may itself be a const-offset GEP — fold one
            // level; anything deeper treats the inner GEP as the base.
            let mut addr = match eval_sym_addr(block, def_pos, *base) {
                Some(a) if a.var.is_none() => a,
                _ => SymAddr {
                    base: *base,
                    var: None,
                    mult: 1,
                    off: 0,
                },
            };
            match offset {
                Operand::Const(c) => {
                    addr.off = addr.off.checked_add(c.to_i64()?)?;
                }
                Operand::Value(off_val) => {
                    let off_inst = block.instructions.get(*def_pos.get(&off_val.0)?)?;
                    match off_inst {
                        Instruction::BinOp {
                            op: IrBinOp::Add,
                            lhs,
                            rhs,
                            ..
                        } => match (lhs, rhs) {
                            (Operand::Value(v), Operand::Const(c))
                            | (Operand::Const(c), Operand::Value(v)) => {
                                if addr.var.is_some() || addr.mult != 1 {
                                    return None; // no affine index sums in v1
                                }
                                addr.var = Some(*v);
                                addr.off = addr.off.checked_add(c.to_i64()?)?;
                            }
                            _ => return None,
                        },
                        // A bare variable index: `GEP(base, iv)` (mult 1).
                        _ if addr.var.is_none() && addr.mult == 1 => {
                            addr.var = Some(*off_val);
                        }
                        _ => return None,
                    }
                }
            }
            Some(addr)
        }
        _ => Some(SymAddr {
            base: ptr,
            var: None,
            mult: 1,
            off: 0,
        }),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Lane-type dispatch
// ─────────────────────────────────────────────────────────────────────────

/// The intrinsic opcode set for one (lane type, width) family.
#[derive(Clone, Copy)]
struct VecFamily {
    load: IntrinsicOp,
    store: IntrinsicOp,
    broadcast: IntrinsicOp,
    zero: IntrinsicOp,
    /// 2-lane gather (I64/F64 only in v1). None ⇒ gathers reject the seed.
    pack2: Option<IntrinsicOp>,
    /// 4-lane gather (I32x4 only in v1).
    pack4: Option<IntrinsicOp>,
    /// Lane extract producing the exact scalar lane type. None ⇒ seeds
    /// needing extracts are rejected (all 256-bit families, F32x4, I16x8,
    /// I8x16 — no exact extract intrinsic exists for them in v1).
    extract: Option<IntrinsicOp>,
    /// Lane byte size.
    size: u64,
}

fn family_for(ty: IrType, width: usize) -> Option<VecFamily> {
    let avx2 = x86_avx2_available_pub();
    match (ty, width) {
        (IrType::F64, 2) => Some(VecFamily {
            load: IntrinsicOp::VecLoadF64x2,
            store: IntrinsicOp::VecStoreF64x2,
            broadcast: IntrinsicOp::VecBroadcastF64x2,
            zero: IntrinsicOp::VecZeroF64x2,
            pack2: Some(IntrinsicOp::VecPackF64x2),
            pack4: None,
            extract: Some(IntrinsicOp::VecExtractLaneF64x2),
            size: 8,
        }),
        (IrType::F64, 4) if avx2 => Some(VecFamily {
            load: IntrinsicOp::VecLoadF64x4,
            store: IntrinsicOp::VecStoreF64x4,
            broadcast: IntrinsicOp::VecBroadcastF64x4,
            zero: IntrinsicOp::VecZeroF64x4,
            pack2: None,
            pack4: None,
            extract: None,
            size: 8,
        }),
        (IrType::F32, 4) => Some(VecFamily {
            load: IntrinsicOp::VecLoadF32x4,
            store: IntrinsicOp::VecStoreF32x4,
            broadcast: IntrinsicOp::VecBroadcastF32x4,
            zero: IntrinsicOp::VecZeroF32x4,
            pack2: None,
            pack4: None,
            extract: None,
            size: 4,
        }),
        (IrType::F32, 8) if avx2 => Some(VecFamily {
            load: IntrinsicOp::VecLoadF32x8,
            store: IntrinsicOp::VecStoreF32x8,
            broadcast: IntrinsicOp::VecBroadcastF32x8,
            zero: IntrinsicOp::VecZeroF32x8,
            pack2: None,
            pack4: None,
            extract: None,
            size: 4,
        }),
        (IrType::I32 | IrType::U32, 4) => Some(VecFamily {
            load: IntrinsicOp::VecLoadI32x4,
            store: IntrinsicOp::VecStoreI32x4,
            broadcast: IntrinsicOp::VecBroadcastI32x4,
            zero: IntrinsicOp::VecZeroI32x4,
            pack2: None,
            pack4: Some(IntrinsicOp::VecPackI32x4),
            extract: Some(IntrinsicOp::VecExtractLaneI32x4),
            size: 4,
        }),
        (IrType::I32 | IrType::U32, 8) if avx2 => Some(VecFamily {
            load: IntrinsicOp::VecLoadI32x8,
            store: IntrinsicOp::VecStoreI32x8,
            broadcast: IntrinsicOp::VecBroadcastI32x8,
            zero: IntrinsicOp::VecZeroI32x8,
            pack2: None,
            pack4: None,
            extract: None,
            size: 4,
        }),
        (IrType::I64 | IrType::U64, 2) => Some(VecFamily {
            load: IntrinsicOp::VecLoadI64x2,
            store: IntrinsicOp::VecStoreI64x2,
            broadcast: IntrinsicOp::VecBroadcastI64x2,
            zero: IntrinsicOp::VecZeroI64x2,
            pack2: Some(IntrinsicOp::VecPackI64x2),
            pack4: None,
            extract: Some(IntrinsicOp::VecExtractLaneI64x2),
            size: 8,
        }),
        (IrType::I64 | IrType::U64, 4) if avx2 => Some(VecFamily {
            load: IntrinsicOp::VecLoadI64x4,
            store: IntrinsicOp::VecStoreI64x4,
            broadcast: IntrinsicOp::VecBroadcastI64x4,
            zero: IntrinsicOp::VecZeroI64x4,
            pack2: None,
            pack4: None,
            extract: None,
            size: 8,
        }),
        (IrType::I16 | IrType::U16, 8) => Some(VecFamily {
            load: IntrinsicOp::VecLoadI16x8,
            store: IntrinsicOp::VecStoreI16x8,
            broadcast: IntrinsicOp::VecBroadcastI16x8,
            // No VecZeroI16x8 exists; a zero splat goes through the
            // broadcast of an integer zero constant (GPR-domain — safe).
            zero: IntrinsicOp::VecBroadcastI16x8,
            pack2: None,
            pack4: None,
            extract: None,
            size: 2,
        }),
        (IrType::I8 | IrType::U8, 16) => Some(VecFamily {
            load: IntrinsicOp::VecLoadI8x16,
            store: IntrinsicOp::VecStoreI8x16,
            broadcast: IntrinsicOp::VecBroadcastI8x16,
            zero: IntrinsicOp::VecBroadcastI8x16,
            pack2: None,
            pack4: None,
            extract: None,
            size: 1,
        }),
        _ => None,
    }
}

/// Map a supported lane binop to its packed intrinsic. None for ops with
/// no exact packed form (div/rem/shifts/bit-test/rotates, i64 Muls —
/// there is no SSE2/AVX2 pmulq).
fn packed_binop(op: IrBinOp, ty: IrType, width: usize) -> Option<IntrinsicOp> {
    match (ty, width) {
        (IrType::F64, 2) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddF64x2),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubF64x2),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulF64x2),
            IrBinOp::SDiv | IrBinOp::UDiv => Some(IntrinsicOp::VecDivF64x2),
            _ => None,
        },
        (IrType::F64, 4) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddF64x4),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubF64x4),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulF64x4),
            IrBinOp::SDiv | IrBinOp::UDiv => Some(IntrinsicOp::VecDivF64x4),
            _ => None,
        },
        (IrType::F32, 4) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddF32x4),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubF32x4),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulF32x4),
            IrBinOp::SDiv | IrBinOp::UDiv => Some(IntrinsicOp::VecDivF32x4),
            _ => None,
        },
        (IrType::F32, 8) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddF32x8),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubF32x8),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulF32x8),
            IrBinOp::SDiv | IrBinOp::UDiv => Some(IntrinsicOp::VecDivF32x8),
            _ => None,
        },
        (IrType::I32 | IrType::U32, 4) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI32x4),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI32x4),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulI32x4),
            IrBinOp::And => Some(IntrinsicOp::VecAndI32x4),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI32x4),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI32x4),
            _ => None,
        },
        (IrType::I32 | IrType::U32, 8) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI32x8),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI32x8),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulI32x8),
            IrBinOp::And => Some(IntrinsicOp::VecAndI32x8),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI32x8),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI32x8),
            _ => None,
        },
        (IrType::I64 | IrType::U64, 2) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI64x2),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI64x2),
            IrBinOp::And => Some(IntrinsicOp::VecAndI64x2),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI64x2),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI64x2),
            _ => None,
        },
        (IrType::I64 | IrType::U64, 4) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI64x4),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI64x4),
            IrBinOp::And => Some(IntrinsicOp::VecAndI64x4),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI64x4),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI64x4),
            _ => None,
        },
        (IrType::I16 | IrType::U16, 8) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI16x8),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI16x8),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulI16x8),
            IrBinOp::And => Some(IntrinsicOp::VecAndI16x8),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI16x8),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI16x8),
            _ => None,
        },
        (IrType::I8 | IrType::U8, 16) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI8x16),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI8x16),
            IrBinOp::And => Some(IntrinsicOp::VecAndI8x16),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI8x16),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI8x16),
            _ => None,
        },
        _ => None,
    }
}

fn is_commutative(op: IrBinOp) -> bool {
    matches!(
        op,
        IrBinOp::Add | IrBinOp::Mul | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
    )
}

/// Lane types the store-seed collector accepts.
fn supported_store_ty(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::F64
            | IrType::F32
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::I16
            | IrType::U16
            | IrType::I8
            | IrType::U8
    )
}

// ─────────────────────────────────────────────────────────────────────────
// Pack graph
// ─────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum PackKind {
    /// N consecutive-address loads → one vector load. `ptrs[i]` is the
    /// pointer VALUE of lane i (lane 0's anchors the access); `offs[i]`
    /// its stream byte offset (offs[i] = offs[0] + i*size — re-asserted
    /// by the run builder).
    MemLoad { ptrs: Vec<Value>, offs: Vec<i64> },
    /// Same-op binop lanes. `ty` is the lane type (== the binop type).
    BinOp {
        op: IrBinOp,
        ty: IrType,
        vec_op: IntrinsicOp,
        lhs: usize,
        rhs: usize,
    },
    /// One scalar operand broadcast to all lanes (`is_zero` selects the
    /// cheaper VecZero lowering at emit time).
    Splat { src: Operand, is_zero: bool },
    /// 2-lane gather (VecPackI64x2 / VecPackF64x2).
    Gather2 {
        op: IntrinsicOp,
        lo: Operand,
        hi: Operand,
    },
    /// 4-lane gather (VecPackI32x4).
    Gather4 { ops: Vec<Operand> },
}

#[derive(Clone, Debug)]
struct Pack {
    kind: PackKind,
    /// The original scalar lane VALUES this pack replaces (empty for
    /// splats/gathers, which replace nothing).
    lane_vals: Vec<Value>,
    /// Schedule slot (original instruction index the vector op — and its
    /// extracts — are inserted before). `usize::MAX` until resolved.
    sched: usize,
    /// Emission order within one insertion slot (topological depth).
    order: usize,
}

struct SeedCandidate {
    /// Store instruction indices, address-ordered (lane i at stream offset
    /// off0 + i*size).
    store_idx: Vec<usize>,
    /// Stored-value operands, address-ordered.
    lane_ops: Vec<Operand>,
    ty: IrType,
    /// Lowest-offset store's pointer value — the vector store anchor.
    anchor_ptr: Value,
    width: usize,
}

// ─────────────────────────────────────────────────────────────────────────
// The pass
// ─────────────────────────────────────────────────────────────────────────

/// Entry point: run basic-block SLP on one function. Returns the number of
/// seeds vectorized.
pub(crate) fn run_bb_slp(func: &mut IrFunction) -> usize {
    if !x86_simd_available_pub() {
        return 0;
    }
    if std::env::var("CCC_NO_BB_SLP").is_ok() {
        return 0;
    }
    let debug = std::env::var("LCCC_DEBUG_SLP").is_ok();
    let mut total = 0usize;
    // Fixpoint: each fired seed rewrites its block, so re-scan afterwards.
    for _round in 0..24 {
        let mut fired = 0usize;
        for b in 0..func.blocks.len() {
            while slp_block_once(func, b, debug) == 1 {
                fired += 1;
            }
        }
        total += fired;
        if fired == 0 {
            break;
        }
    }
    if total > 0 {
        crate::passes::dce::eliminate_dead_code(func);
    }
    total
}

struct BlockCtx<'a> {
    block: &'a BasicBlock,
    def_pos: FxHashMap<u32, usize>,
    /// Use positions (instruction indices; `usize::MAX` = terminator) of
    /// each value IN THIS BLOCK.
    uses: FxHashMap<u32, Vec<usize>>,
    /// Values with uses in OTHER blocks of the function (phi edges,
    /// later-block consumers). A packed lane in this set cannot be
    /// replaced by a block-local extract — reject its seed.
    external_uses: FxHashSet<u32>,
}

fn build_ctx(func: &IrFunction, block_idx: usize) -> BlockCtx<'_> {
    let block = &func.blocks[block_idx];
    let mut def_pos: FxHashMap<u32, usize> = FxHashMap::default();
    for (i, inst) in block.instructions.iter().enumerate() {
        if let Some(d) = inst.dest() {
            def_pos.insert(d.0, i);
        }
    }
    // Use collection goes through the CANONICAL `for_each_used_value`
    // walker — the single authority the IR maintains for "every value this
    // instruction consumes". The hand-rolled per-variant match this pass
    // used originally missed Call/CallIndirect arguments, Phi incomings,
    // InlineAsm inputs, the atomic val operands and the static-chain/
    // second-return setters: a lane flowing into any of those would have
    // had its definition removed while the use dangled (backend ICE or
    // silent miscompile). Phis in this block note their incomings at the
    // phi's position; a self-loop backedge use of a lane is therefore
    // rejected by rule (a) — conservative, never unsound.
    let mut uses: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (i, inst) in block.instructions.iter().enumerate() {
        inst.for_each_used_value(|vid| uses.entry(vid).or_default().push(i));
    }
    block
        .terminator
        .for_each_used_value(|vid| uses.entry(vid).or_default().push(usize::MAX));
    // Cross-block uses: every OTHER block's instructions/terminators via
    // the same canonical walker — including the Phi nodes the old scan
    // missed entirely (rule (b) was vacuous for phi edges).
    let mut external_uses: FxHashSet<u32> = FxHashSet::default();
    for (bi, other) in func.blocks.iter().enumerate() {
        if bi == block_idx {
            continue;
        }
        for inst in &other.instructions {
            inst.for_each_used_value(|vid| {
                external_uses.insert(vid);
            });
        }
        other.terminator.for_each_used_value(|vid| {
            external_uses.insert(vid);
        });
    }
    BlockCtx {
        block,
        def_pos,
        uses,
        external_uses,
    }
}

/// Does this instruction write memory? Delegates to the canonical
/// `Instruction::may_write_memory`, which covers every IR variant
/// including the `Intrinsic` store class (`VecStore*` via `dest_ptr` OR
/// via `args[1]`, `Movnt*`, `Storedqu`, jump-buffer ops).
///
/// The previous hand-rolled list had NO `Intrinsic` arm at all: the
/// `VecStore*` this very pass emits (and the loop vectorizer's, already
/// sitting in the block when BB-SLP re-scans) was invisible to rules (c)
/// and (d) — a second seed could legally vectorize loads ACROSS an
/// already-emitted vector store that writes the loaded bytes.
fn is_memory_write(inst: &Instruction) -> bool {
    inst.may_write_memory()
}

/// Does this instruction access memory (read or write)? In addition to
/// the writes above: scalar/atomic loads, va_copy, and the intrinsic
/// READ class (`VecLoad*`, widening and explicit-SIMD loads —
/// `IntrinsicOp::may_read_memory`). The vector loads are exactly as
/// observable as scalar ones for the store-interval rule (d).
fn is_memory_access(inst: &Instruction) -> bool {
    is_memory_write(inst)
        || matches!(
            inst,
            Instruction::Load { .. } | Instruction::AtomicLoad { .. } | Instruction::VaCopy { .. }
        )
        || matches!(
            inst,
            Instruction::Intrinsic { op, .. } if op.may_read_memory()
        )
}

/// Collect store seeds: runs of stores on one address stream with
/// consecutive offsets at the lane stride, longest first.
fn collect_seed_candidates(ctx: &BlockCtx) -> Vec<SeedCandidate> {
    let block = ctx.block;
    let avx2 = x86_avx2_available_pub();
    // (base, var, mult, ty) → Vec<(off, store_idx, val, ptr)>
    let mut streams: FxHashMap<(u32, Option<u32>, u64, IrType), Vec<(i64, usize, Operand, Value)>> =
        FxHashMap::default();
    for (i, inst) in block.instructions.iter().enumerate() {
        let Instruction::Store {
            val,
            ptr,
            ty,
            seg_override,
            volatile,
        } = inst
        else {
            continue;
        };
        if *seg_override != AddressSpace::Default || *volatile || !supported_store_ty(*ty) {
            continue;
        }
        let Some(addr) = eval_sym_addr(block, &ctx.def_pos, *ptr) else {
            continue;
        };
        streams
            .entry((addr.base.0, addr.var.map(|v| v.0), addr.mult, *ty))
            .or_default()
            .push((addr.off, i, val.clone(), *ptr));
    }
    let mut candidates = Vec::new();
    for ((_, _, _, ty), mut entries) in streams {
        if entries.len() < 2 {
            continue;
        }
        entries.sort_by_key(|(off, idx, _, _)| (*off, *idx));
        let size = ty.size() as i64;
        if size <= 0 {
            continue;
        }
        // Split into consecutive runs at stride == size. A duplicate
        // offset (store overwritten by a later store to the same address)
        // ends the current run; the earlier store stays scalar.
        let mut run: Vec<(i64, usize, Operand, Value)> = Vec::new();
        let mut flush = |run: &mut Vec<(i64, usize, Operand, Value)>,
                         candidates: &mut Vec<SeedCandidate>| {
            if run.len() >= 2 {
                let mut offs: Vec<i64> = run.iter().map(|(o, ..)| *o).collect();
                offs.dedup();
                if offs.len() == run.len() {
                    if let Some(width) = preferred_width(run.len(), size as u64, ty, avx2) {
                        let store_idx: Vec<usize> =
                            run[..width].iter().map(|(_, i, _, _)| *i).collect();
                        let lane_ops: Vec<Operand> =
                            run[..width].iter().map(|(_, _, v, _)| v.clone()).collect();
                        candidates.push(SeedCandidate {
                            store_idx,
                            lane_ops,
                            ty,
                            anchor_ptr: run[0].3,
                            width,
                        });
                    }
                }
            }
            run.clear();
        };
        for entry in &entries {
            if let Some(last) = run.last() {
                // checked: an offset delta that does not fit i64 cannot be
                // the lane stride (debug builds would panic on overflow).
                if entry.0.checked_sub(last.0) != Some(size) {
                    flush(&mut run, &mut candidates);
                }
            }
            run.push(entry.clone());
        }
        flush(&mut run, &mut candidates);
    }
    // Longest chains first; ties by earliest position.
    candidates.sort_by(|a, b| {
        b.lane_ops
            .len()
            .cmp(&a.lane_ops.len())
            .then(a.store_idx[0].cmp(&b.store_idx[0]))
    });
    candidates
}

/// Pick the vector width (in lanes) for a run of `len` consecutive lanes
/// of `size` bytes: the largest register class the FAMILY TABLE actually
/// provides — 32-byte AVX2 first, then 16-byte SSE2 — capped by the run
/// length. Family-aware: the byte/halfword families currently stop at
/// 16 lanes, so a 32-byte byte-run falls back to I8x16 instead of being
/// dropped outright (the old `preferred_width` returned 32 lanes for
/// every ≥32-byte run and `family_for` then rejected the candidate —
/// those runs never vectorized at all).
fn preferred_width(len: usize, size: u64, ty: IrType, avx2: bool) -> Option<usize> {
    let w256 = (32 / size) as usize;
    let w128 = (16 / size) as usize;
    if avx2 && len >= w256 && family_for(ty, w256).is_some() {
        Some(w256)
    } else if len >= w128 && family_for(ty, w128).is_some() {
        Some(w128)
    } else {
        None
    }
}

/// Base-value classification for rule (e)'s `restrict` escape.
///
/// C11 6.7.3.1: while a `restrict` pointer parameter P designates an
/// object, every access to that object in the function must be through a
/// lvalue based on P. A base value that is a PARAMETER, a GLOBAL address,
/// or a frame ALLOCA is never "based on" P (entry values and frame/
/// global objects are not computed from P), so an overlap between its
/// accesses and P's would violate the caller's restrict contract — the
/// compiler may assume the two bases never designate overlapping objects.
/// Computed bases (loads, calls, arithmetic results) MAY be based on P
/// (e.g. P stored to memory and reloaded) and get no assumption.
#[derive(Default)]
struct RestrictBases {
    /// ParamRef dests of pointer params carrying `restrict`.
    noalias: FxHashSet<u32>,
    /// Values provably not based on any restrict parameter: parameter
    /// values, global addresses, frame allocas.
    independent: FxHashSet<u32>,
}

impl RestrictBases {
    fn build(func: &IrFunction) -> Self {
        let mut rb = RestrictBases::default();
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::ParamRef {
                        dest, param_idx, ..
                    } => {
                        rb.independent.insert(dest.0);
                        if func.params.get(*param_idx).is_some_and(|p| p.noalias) {
                            rb.noalias.insert(dest.0);
                        }
                    }
                    Instruction::GlobalAddr { dest, .. } | Instruction::Alloca { dest, .. } => {
                        rb.independent.insert(dest.0);
                    }
                    _ => {}
                }
            }
        }
        rb
    }

    /// May the two base values be assumed to designate disjoint objects
    /// under the caller's restrict contracts? Requires distinct values,
    /// one carrying `restrict`, and the other provably not based on it.
    fn disjoint(&self, a: u32, b: u32) -> bool {
        a != b
            && ((self.noalias.contains(&a) && self.independent.contains(&b))
                || (self.noalias.contains(&b) && self.independent.contains(&a)))
    }
}

fn slp_block_once(func: &mut IrFunction, block_idx: usize, debug: bool) -> usize {
    // All analysis under one immutable borrow; the plans are fully owned
    // (no lifetimes into the block), so the mutable rewrite afterwards
    // is borrow-clean.
    let (candidates, plans): (Vec<SeedCandidate>, Vec<Option<Plan>>) = {
        let ctx = build_ctx(func, block_idx);
        let bases = RestrictBases::build(func);
        let candidates = collect_seed_candidates(&ctx);
        let plans = candidates
            .iter()
            .map(|c| build_plan(&ctx, c, &bases))
            .collect();
        (candidates, plans)
    };
    for (cand, plan) in candidates.iter().zip(plans) {
        if let Some(plan) = plan {
            if apply_plan(func, block_idx, &plan, cand, debug) {
                return 1;
            }
        }
    }
    0
}

// ─────────────────────────────────────────────────────────────────────────
// Plan construction (pack graph + legality + cost)
// ─────────────────────────────────────────────────────────────────────────

struct Plan {
    packs: Vec<Pack>,
    /// Root pack index (the pack feeding the seed store).
    root: usize,
    /// Original positions removed by the rewrite (lane ops, seed stores).
    removed: FxHashSet<usize>,
    /// Lane value id → (pack index, lane index) for extract placement.
    extract_of: FxHashMap<u32, (usize, usize)>,
    /// Net benefit (≥ 1 required).
    benefit: i64,
}

/// Bottom-up pack builder over operand lanes.
///
/// Returns the pack index, or None when the lanes cannot be represented.
/// SSA def-before-use within a block makes cycles impossible; the depth
/// guard covers pathological shapes only.
fn build_pack(
    ctx: &BlockCtx,
    lanes: &[Operand],
    ty: IrType,
    width: usize,
    fam: &VecFamily,
    packs: &mut Vec<Pack>,
    dedup: &mut FxHashMap<Vec<LaneKey>, usize>,
    depth: usize,
) -> Option<usize> {
    if packs.len() >= MAX_PACKS || depth > 16 {
        return None;
    }
    let key: Vec<LaneKey> = lanes.iter().map(lane_key).collect();
    if let Some(&idx) = dedup.get(&key) {
        return Some(idx);
    }
    let block = ctx.block;

    // 1. Splat: all lanes the same operand.
    if lanes.windows(2).all(|w| w[0] == w[1]) {
        let src = lanes[0].clone();
        let is_zero = match &src {
            Operand::Const(c) => c.to_i64() == Some(0),
            _ => false,
        };
        // FP non-zero constant splats have no audited broadcast path for
        // Const operands (FP constants do not go through the GPR staging
        // the broadcast lowerings assume) — reject, they stay scalar.
        if matches!(ty, IrType::F32 | IrType::F64) && !is_zero {
            return None;
        }
        let idx = packs.len();
        packs.push(Pack {
            kind: PackKind::Splat { src, is_zero },
            lane_vals: Vec::new(),
            sched: usize::MAX,
            order: 0,
        });
        dedup.insert(key, idx);
        return Some(idx);
    }

    // All-value path from here down (loads / binops need defs).
    let as_values: Option<Vec<Value>> = lanes
        .iter()
        .map(|l| match l {
            Operand::Value(v) => Some(*v),
            Operand::Const(_) => None,
        })
        .collect();

    if let Some(vals) = as_values {
        // 1b. Sub-word promotion rewrite: C integer promotion leaves sub-word
        // lane math as `trunc(zext(a) OP zext(b))` in I32. For two's-
        // complement Add/Sub/Mul/And/Or/Xor the low lane bits of the
        // promoted op are EXACTLY the sub-word packed op (an unsigned
        // promoted product always fits I32; a signed one fits in 2^30;
        // pmullw/paddw/... compute the wrapping low bits — identical to
        // truncating the exact promoted result). Strip the
        // truncation/widening casts and pack at the store's lane width.
        if matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
            let truncs_ok = vals.iter().all(|v| {
                matches!(
                    ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]),
                    Some(Instruction::Cast { from_ty, to_ty, .. })
                        if *to_ty == ty && matches!(from_ty, IrType::I32 | IrType::I64)
                )
            });
            if truncs_ok {
                // The truncated sources must be same-op I32 binops whose
                // operands widen sub-word values of the same lane type.
                let mut promoted: Vec<Value> = Vec::with_capacity(width);
                let mut ok = true;
                for v in &vals {
                    let i = ctx.def_pos[&v.0];
                    let Instruction::Cast { src, .. } = &block.instructions[i] else {
                        unreachable!()
                    };
                    let Operand::Value(sv) = src else {
                        ok = false;
                        break;
                    };
                    match ctx.def_pos.get(&sv.0).map(|&j| &block.instructions[j]) {
                        Some(Instruction::BinOp { op, ty: bty, .. })
                            if *bty == IrType::I32
                                && matches!(
                                    op,
                                    IrBinOp::Add
                                        | IrBinOp::Sub
                                        | IrBinOp::Mul
                                        | IrBinOp::And
                                        | IrBinOp::Or
                                        | IrBinOp::Xor
                                ) =>
                        {
                            promoted.push(*sv);
                        }
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    // Strip the widening casts off the binop operands.
                    let mut op_a: Vec<Operand> = Vec::with_capacity(width);
                    let mut op_b: Vec<Operand> = Vec::with_capacity(width);
                    let mut uniform_op: Option<IrBinOp> = None;
                    let mut ops_ok = true;
                    for pv in &promoted {
                        let j = ctx.def_pos[&pv.0];
                        let Instruction::BinOp { op, lhs, rhs, .. } = &block.instructions[j] else {
                            unreachable!()
                        };
                        match uniform_op {
                            None => uniform_op = Some(*op),
                            Some(o0) if o0 == *op => {}
                            _ => {
                                ops_ok = false;
                                break;
                            }
                        }
                        let strip = |o: &Operand| -> Option<Operand> {
                            match o {
                                // Value operands must be widening casts of
                                // the lane type (the zext/sext C promotion).
                                Operand::Value(w) => {
                                    match ctx.def_pos.get(&w.0).map(|&k| &block.instructions[k]) {
                                        Some(Instruction::Cast {
                                            src,
                                            from_ty,
                                            to_ty,
                                            ..
                                        }) if *from_ty == ty
                                            && matches!(*to_ty, IrType::I32 | IrType::I64) =>
                                        {
                                            Some(src.clone())
                                        }
                                        _ => None,
                                    }
                                }
                                // Constant operands must fit the sub-word
                                // lane (the low bits are then the sub-word
                                // constant).
                                Operand::Const(c) => match c.to_i64() {
                                    Some(cv)
                                        if cv >= -(1 << (8 * ty.size() - 1))
                                            && cv < (1 << (8 * ty.size())) =>
                                    {
                                        Some(Operand::Const(match ty {
                                            IrType::I8 | IrType::U8 => IrConst::I8(cv as i8),
                                            _ => IrConst::I16(cv as i16),
                                        }))
                                    }
                                    _ => None,
                                },
                            }
                        };
                        match (strip(lhs), strip(rhs)) {
                            (Some(a), Some(b)) => {
                                op_a.push(a);
                                op_b.push(b);
                            }
                            _ => {
                                ops_ok = false;
                                break;
                            }
                        }
                    }
                    if ops_ok && uniform_op.is_some() {
                        let op = uniform_op.unwrap();
                        if let Some(vec_op) = packed_binop(op, ty, width) {
                            // Recurse at the sub-word width; operand packs
                            // see the raw sub-word loads/values.
                            if let (Some(lhs), Some(rhs)) = (
                                build_pack(ctx, &op_a, ty, width, fam, packs, dedup, depth + 1),
                                build_pack(ctx, &op_b, ty, width, fam, packs, dedup, depth + 1),
                            ) {
                                let idx = packs.len();
                                packs.push(Pack {
                                    kind: PackKind::BinOp {
                                        op,
                                        ty,
                                        vec_op,
                                        lhs,
                                        rhs,
                                    },
                                    // The removed lanes are the truncating
                                    // casts; the widening casts and promoted
                                    // muls stay (they may have other uses)
                                    // and become dead via DCE when not.
                                    lane_vals: vals.clone(),
                                    sched: usize::MAX,
                                    order: 0,
                                });
                                dedup.insert(key, idx);
                                return Some(idx);
                            }
                        }
                    }
                }
            }
        }

        // 2. Consecutive-address loads of the exact lane type.
        let loads_ok = vals.iter().all(|v| {
            matches!(ctx.def_pos.get(&v.0), Some(&i)
                if matches!(&block.instructions[i],
                    Instruction::Load { ty: lt, seg_override, volatile, .. }
                        if *lt == ty && *seg_override == AddressSpace::Default && !*volatile))
        });
        if loads_ok {
            let mut addrs = Vec::with_capacity(width);
            let mut ptrs = Vec::with_capacity(width);
            let mut ok = true;
            for v in &vals {
                let i = ctx.def_pos[&v.0];
                let Instruction::Load { ptr, .. } = &block.instructions[i] else {
                    ok = false;
                    break;
                };
                match eval_sym_addr(block, &ctx.def_pos, *ptr) {
                    Some(a) => {
                        addrs.push(a);
                        ptrs.push(*ptr);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok
                && addrs.windows(2).all(|w| {
                    w[0].base == w[1].base
                        && w[0].var == w[1].var
                        && w[0].mult == w[1].mult
                        && w[1].off.checked_sub(w[0].off) == Some(fam.size as i64)
                })
            {
                let idx = packs.len();
                packs.push(Pack {
                    kind: PackKind::MemLoad {
                        ptrs,
                        offs: addrs.iter().map(|a| a.off).collect(),
                    },
                    lane_vals: vals,
                    sched: usize::MAX,
                    order: 0,
                });
                dedup.insert(key, idx);
                return Some(idx);
            }
        }

        // 3. Same-op, same-type binop lanes. Operand sides flow into the
        // recursion as Operands — an all-same-const side becomes a splat
        // (`a[i] * 2`), anything unbuildable degrades to a gather.
        let mut first: Option<(IrBinOp, IrType)> = None;
        let mut uniform = true;
        for v in &vals {
            match ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]) {
                Some(Instruction::BinOp { op, ty: bty, .. }) => match first {
                    None => first = Some((*op, *bty)),
                    Some((op0, ty0)) if op0 == *op && ty0 == *bty => {}
                    _ => {
                        uniform = false;
                        break;
                    }
                },
                _ => {
                    uniform = false;
                    break;
                }
            }
        }
        if uniform && first.is_some() {
            let (op, bty) = first.unwrap();
            if bty == ty {
                if let Some(vec_op) = packed_binop(op, ty, width) {
                    let mut lhs_lanes: Vec<Operand> = Vec::with_capacity(width);
                    let mut rhs_lanes: Vec<Operand> = Vec::with_capacity(width);
                    for v in &vals {
                        let i = ctx.def_pos[&v.0];
                        let Instruction::BinOp { lhs, rhs, .. } = &block.instructions[i] else {
                            unreachable!()
                        };
                        lhs_lanes.push(lhs.clone());
                        rhs_lanes.push(rhs.clone());
                    }
                    // Try (lhs, rhs); if an operand side cannot pack and
                    // the op is commutative, retry swapped (`b[i]*a[i]`
                    // spellings normalize).
                    let mut lhs_pack =
                        build_pack(ctx, &lhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                    let mut rhs_pack =
                        build_pack(ctx, &rhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                    if (lhs_pack.is_none() || rhs_pack.is_none()) && is_commutative(op) {
                        let sw_lhs =
                            build_pack(ctx, &rhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                        let sw_rhs =
                            build_pack(ctx, &lhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                        if sw_lhs.is_some() && sw_rhs.is_some() {
                            lhs_pack = sw_lhs;
                            rhs_pack = sw_rhs;
                        }
                    }
                    if let (Some(lhs), Some(rhs)) = (lhs_pack, rhs_pack) {
                        let idx = packs.len();
                        packs.push(Pack {
                            kind: PackKind::BinOp {
                                op,
                                ty,
                                vec_op,
                                lhs,
                                rhs,
                            },
                            lane_vals: vals,
                            sched: usize::MAX,
                            order: 0,
                        });
                        dedup.insert(key, idx);
                        return Some(idx);
                    }
                }
            }
        }
    }

    // 4. Gathers: the lanes stay scalar; a VecPack builds the vector from
    //    the (Value or Const) operands. Only 2-lane I64/F64 and 4-lane
    //    I32 families have gather intrinsics in v1.
    match (width, fam.pack2, fam.pack4) {
        (2, Some(pack_op), _) => {
            let idx = packs.len();
            packs.push(Pack {
                kind: PackKind::Gather2 {
                    op: pack_op,
                    lo: lanes[0].clone(),
                    hi: lanes[1].clone(),
                },
                lane_vals: Vec::new(),
                sched: usize::MAX,
                order: 0,
            });
            dedup.insert(key, idx);
            Some(idx)
        }
        (4, _, Some(_)) => {
            let idx = packs.len();
            packs.push(Pack {
                kind: PackKind::Gather4 {
                    ops: lanes.to_vec(),
                },
                lane_vals: Vec::new(),
                sched: usize::MAX,
                order: 0,
            });
            dedup.insert(key, idx);
            Some(idx)
        }
        _ => None,
    }
}

fn build_plan(ctx: &BlockCtx, cand: &SeedCandidate, bases: &RestrictBases) -> Option<Plan> {
    let block = ctx.block;
    let width = cand.width;
    let fam = family_for(cand.ty, width)?;
    let mut packs: Vec<Pack> = Vec::new();
    let mut dedup: FxHashMap<Vec<LaneKey>, usize> = FxHashMap::default();

    let root = build_pack(
        ctx,
        &cand.lane_ops,
        cand.ty,
        width,
        &fam,
        &mut packs,
        &mut dedup,
        0,
    )?;

    // ── Schedules ──────────────────────────────────────────────────────
    // Op/MemLoad packs: max lane def position (strictly before every
    // consumer — SSA within the block). Splat/gather leaves: min over
    // consumers, resolved by fixpoint.
    for p in packs.iter_mut() {
        if !p.lane_vals.is_empty() {
            p.sched = p
                .lane_vals
                .iter()
                .map(|v| ctx.def_pos[&v.0])
                .max()
                .unwrap_or(0);
        }
    }
    loop {
        let mut changed = false;
        for i in 0..packs.len() {
            if packs[i].sched != usize::MAX {
                continue;
            }
            let mut min_sched = usize::MAX;
            let mut has_consumer = false;
            for (j, other) in packs.iter().enumerate() {
                if j == i {
                    continue;
                }
                let inputs = match &other.kind {
                    PackKind::BinOp { lhs, rhs, .. } => Some((*lhs, *rhs)),
                    _ => None,
                };
                if let Some((l, r)) = inputs {
                    if l == i || r == i {
                        has_consumer = true;
                        if other.sched != usize::MAX {
                            min_sched = min_sched.min(other.sched);
                        }
                    }
                }
            }
            if has_consumer && min_sched != usize::MAX {
                packs[i].sched = min_sched;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Unresolved leaves mean a graph bug (the root always has the seed
    // store as consumer via the tree) — bail.
    if packs.iter().any(|p| p.sched == usize::MAX) {
        return None;
    }
    // Topological emission order within a slot.
    for i in 0..packs.len() {
        packs[i].order = topo_depth(&packs, i);
    }

    // ── Removed positions ──────────────────────────────────────────────
    let mut removed: FxHashSet<usize> = FxHashSet::default();
    for p in &packs {
        for v in &p.lane_vals {
            removed.insert(ctx.def_pos[&v.0]);
        }
    }
    for &i in &cand.store_idx {
        removed.insert(i);
    }

    // Transitive dead-lane cleanup: the promotion rewrite's zext/mul chain
    // (and any other pure lane-feeding computation) whose EVERY use is a
    // removed lane position is itself dead — DCE would remove it anyway.
    // Removing it eagerly here keeps rule (a) from rejecting the seed over
    // uses that will not exist in the rewritten block. Scoped to pure
    // Cast/BinOp feeders with no cross-block use.
    //
    // Gather/splat OPERANDS are exempt: a Gather2/Gather4/Splat pack
    // consumes the ORIGINAL scalar value (the seed store's operand) in
    // the new vector instruction. Its def must survive even when the
    // value's only original use was the removed store — otherwise the
    // emitted VecPack*/VecBroadcast* would reference a deleted value and
    // the backend ICEs ("no register, stack slot, Copy, or GlobalAddr
    // definition"; nbody GLA advance B9).
    let mut keep_operands: FxHashSet<u32> = FxHashSet::default();
    for p in packs.iter() {
        match &p.kind {
            PackKind::Splat { src, .. } => {
                if let Operand::Value(v) = src {
                    keep_operands.insert(v.0);
                }
            }
            PackKind::Gather2 { lo, hi, .. } => {
                for o in [lo, hi] {
                    if let Operand::Value(v) = o {
                        keep_operands.insert(v.0);
                    }
                }
            }
            PackKind::Gather4 { ops } => {
                for o in ops {
                    if let Operand::Value(v) = o {
                        keep_operands.insert(v.0);
                    }
                }
            }
            _ => {}
        }
    }
    loop {
        let mut added = false;
        for (i, inst) in block.instructions.iter().enumerate() {
            if removed.contains(&i) {
                continue;
            }
            if !matches!(inst, Instruction::Cast { .. } | Instruction::BinOp { .. }) {
                continue;
            }
            let Some(d) = inst.dest() else { continue };
            if ctx.external_uses.contains(&d.0) || keep_operands.contains(&d.0) {
                continue;
            }
            let uses = ctx.uses.get(&d.0).map(|us| us.as_slice()).unwrap_or(&[]);
            if uses.is_empty() {
                continue;
            }
            let all_removed = uses
                .iter()
                .all(|&u| u != usize::MAX && removed.contains(&u));
            if all_removed {
                removed.insert(i);
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    // ── Legality ───────────────────────────────────────────────────────
    for p in packs.iter() {
        for v in &p.lane_vals {
            // (b) No cross-block uses of a replaced lane.
            if ctx.external_uses.contains(&v.0) {
                return None;
            }
            // (a) In-block external uses must be scheduled strictly after
            // the pack's vector op (the replacing extract is placed with
            // the vector op). Uses at usize::MAX (terminator) are always
            // after — extracts are pure.
            for &u in ctx.uses.get(&v.0).unwrap_or(&Vec::new()) {
                if u != usize::MAX && !removed.contains(&u) && u <= p.sched {
                    return None;
                }
            }
        }
        // (c) No memory write strictly between the lane loads of a
        // MemLoad pack: an interleaved write could change which lanes see
        // old vs. new values.
        if let PackKind::MemLoad { .. } = &p.kind {
            let positions: Vec<usize> = p.lane_vals.iter().map(|v| ctx.def_pos[&v.0]).collect();
            let lo = *positions.iter().min().unwrap();
            let hi = *positions.iter().max().unwrap();
            for q in lo + 1..hi {
                if is_memory_write(&block.instructions[q]) && !removed.contains(&q) {
                    return None;
                }
            }
        }
    }
    // (d) No memory access strictly between the seed stores other than
    // the removed lanes themselves: the vector store commits all lanes at
    // m_max, so an interleaved reader could observe a different
    // half-stored state.
    let m_min = *cand.store_idx.iter().min().unwrap();
    let m_max = *cand.store_idx.iter().max().unwrap();
    for q in m_min + 1..m_max {
        if is_memory_access(&block.instructions[q]) && !removed.contains(&q) {
            return None;
        }
    }

    // (e) Seed-store vs vector-load lane hazard. The vector load re-reads
    // EVERY lane at the last lane definition (M); the seed stores all move
    // to m_max > M. A lane whose ORIGINAL load position P_i follows a seed
    // store W (W < P_i) that writes bytes the lane reads therefore
    // observed the POST-store value in the original program but reads the
    // PRE-store value after the rewrite (the store is deferred past M).
    // Precisely: same symbolic stream (base, var, mult) ⇒ compare the byte
    // ranges; different streams may alias at runtime ⇒ any W before P_i
    // rejects. This is the store→load forwarding shape
    // (`q[1] = t; u = q[1]; q[2] = u;`), which rules (c)/(d) cannot see
    // because the offending store/load are both removed instructions.
    if packs
        .iter()
        .any(|p| matches!(p.kind, PackKind::MemLoad { .. }))
    {
        // The seed stream and its lowest offset: anchor = lane-0 store's
        // pointer; store si covers [off0 + si*size, +size).
        let seed_addr = eval_sym_addr(block, &ctx.def_pos, cand.anchor_ptr);
        let seed_stream = seed_addr.as_ref().map(|a| (a.base, a.var, a.mult));
        for p in packs.iter() {
            let PackKind::MemLoad { ptrs, offs, .. } = &p.kind else {
                continue;
            };
            let lane_stream =
                eval_sym_addr(block, &ctx.def_pos, ptrs[0]).map(|a| (a.base, a.var, a.mult));
            for (li, &v) in p.lane_vals.iter().enumerate() {
                let lane_pos = ctx.def_pos[&v.0];
                for (si, &w) in cand.store_idx.iter().enumerate() {
                    if w >= lane_pos {
                        continue; // lane loads before this store: no hazard
                    }
                    let may_hit = match (&seed_addr, &lane_stream) {
                        // Same stream: byte-range overlap (i128 so the
                        // pathological-offset subtraction cannot wrap).
                        (Some(sa), Some(_)) if seed_stream == lane_stream => {
                            let store_off = sa.off as i128 + si as i128 * fam.size as i128;
                            let lane_off = offs[li] as i128;
                            (store_off - lane_off).abs() < fam.size as i128
                        }
                        // Restrict escape (C11 6.7.3.1): distinct bases
                        // where one carries `restrict` and the other is
                        // provably not based on it (param/global/alloca)
                        // may be assumed disjoint — the caller's contract
                        // forbids the overlap the hazard would need.
                        (Some(sa), Some((lb_base, _, _)))
                            if bases.disjoint(sa.base.0, lb_base.0) =>
                        {
                            false
                        }
                        _ => true, // distinct or opaque streams: may alias
                    };
                    if may_hit {
                        return None;
                    }
                }
            }
        }
    }

    // ── Extract map ────────────────────────────────────────────────────
    let mut extract_of: FxHashMap<u32, (usize, usize)> = FxHashMap::default();
    for (pi, p) in packs.iter().enumerate() {
        for (li, v) in p.lane_vals.iter().enumerate() {
            let has_external = ctx
                .uses
                .get(&v.0)
                .map(|us| us.iter().any(|&u| u == usize::MAX || !removed.contains(&u)))
                .unwrap_or(false);
            if has_external {
                // Only families with an exact lane-extract intrinsic can
                // service external uses in v1.
                if fam.extract.is_none() {
                    return None;
                }
                extract_of.insert(v.0, (pi, li));
            }
        }
    }

    // ── Cost model ─────────────────────────────────────────────────────
    let mut benefit: i64 = width as i64 - 1; // seed stores: W → 1
    for p in &packs {
        match &p.kind {
            PackKind::MemLoad { .. } | PackKind::BinOp { .. } => {
                benefit += width as i64 - 1;
            }
            PackKind::Splat { is_zero, .. } => {
                benefit -= if *is_zero { 1 } else { 2 };
            }
            PackKind::Gather2 { .. } => benefit -= 3,
            PackKind::Gather4 { .. } => benefit -= 7,
        }
    }
    benefit -= extract_of.len() as i64;
    if benefit < 1 {
        return None;
    }

    Some(Plan {
        packs,
        root,
        removed,
        extract_of,
        benefit,
    })
}

fn topo_depth(packs: &[Pack], idx: usize) -> usize {
    match &packs[idx].kind {
        PackKind::BinOp { lhs, rhs, .. } => {
            1 + topo_depth(packs, *lhs).max(topo_depth(packs, *rhs))
        }
        _ => 0,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Rewrite
// ─────────────────────────────────────────────────────────────────────────

fn apply_plan(
    func: &mut IrFunction,
    block_idx: usize,
    plan: &Plan,
    cand: &SeedCandidate,
    debug: bool,
) -> bool {
    let width = cand.width;
    let fam = match family_for(cand.ty, width) {
        Some(f) => f,
        None => return false,
    };

    // New SSA values: one vector result per pack, one per extract.
    let mut vec_dest: Vec<Value> = Vec::with_capacity(plan.packs.len());
    for _ in 0..plan.packs.len() {
        vec_dest.push(Value(func.next_value_id));
        func.next_value_id += 1;
    }
    let mut extract_dest: FxHashMap<u32, Value> = FxHashMap::default();
    for (&vid, _) in &plan.extract_of {
        extract_dest.insert(vid, Value(func.next_value_id));
        func.next_value_id += 1;
    }

    // Insertion batches: (position, order, instruction). Sorted by
    // (position, order); dependencies land first within a slot.
    let mut inserts: Vec<(usize, usize, Instruction)> = Vec::new();
    for (pi, p) in plan.packs.iter().enumerate() {
        let dest = vec_dest[pi];
        let inst = match &p.kind {
            PackKind::MemLoad { ptrs, .. } => Instruction::Intrinsic {
                dest: Some(dest),
                op: fam.load,
                dest_ptr: None,
                // Lane 0's pointer anchors the access; the vector reads
                // exactly [off0, off0 + width*size) — the union of the
                // scalar lane reads, never more.
                args: vec![Operand::Value(ptrs[0]), Operand::Const(IrConst::I64(0))],
            },
            PackKind::BinOp {
                vec_op, lhs, rhs, ..
            } => Instruction::Intrinsic {
                dest: Some(dest),
                op: *vec_op,
                dest_ptr: None,
                args: vec![
                    Operand::Value(vec_dest[*lhs]),
                    Operand::Value(vec_dest[*rhs]),
                ],
            },
            PackKind::Splat { src, is_zero } => Instruction::Intrinsic {
                dest: Some(dest),
                op: if *is_zero { fam.zero } else { fam.broadcast },
                dest_ptr: None,
                args: vec![src.clone()],
            },
            PackKind::Gather2 { op, lo, hi } => Instruction::Intrinsic {
                dest: Some(dest),
                op: *op,
                dest_ptr: None,
                args: vec![lo.clone(), hi.clone()],
            },
            PackKind::Gather4 { ops } => Instruction::Intrinsic {
                dest: Some(dest),
                op: IntrinsicOp::VecPackI32x4,
                dest_ptr: None,
                args: ops.clone(),
            },
        };
        inserts.push((p.sched, p.order, inst));
        // Extracts ride with their pack (pure reads; placement with the
        // vector op is always before every surviving use).
        for (li, v) in p.lane_vals.iter().enumerate() {
            if let Some(&xdest) = extract_dest.get(&v.0) {
                inserts.push((
                    p.sched,
                    p.order + 1 + li,
                    Instruction::Intrinsic {
                        dest: Some(xdest),
                        op: fam.extract.unwrap(),
                        dest_ptr: None,
                        args: vec![
                            Operand::Value(dest),
                            Operand::Const(IrConst::I32(li as i32)),
                        ],
                    },
                ));
            }
        }
    }
    // The seed store, anchored at the lowest-offset store's pointer.
    // `dest_ptr` mirrors the loop vectorizer's construction so the
    // canonical `Instruction::may_write_memory` classifies the store as a
    // write (its contract keys on `dest_ptr`, with the `VecStore*`
    // args-form supplement); the backend's `emit_vec_store_addr` prefers
    // the 3-arg form and ignores the redundant `dest_ptr`.
    inserts.push((
        *cand.store_idx.iter().max().unwrap(),
        10_000,
        Instruction::Intrinsic {
            dest: None,
            op: fam.store,
            dest_ptr: Some(cand.anchor_ptr),
            args: vec![
                Operand::Value(vec_dest[plan.root]),
                Operand::Value(cand.anchor_ptr),
                Operand::Const(IrConst::I64(0)),
            ],
        },
    ));

    // Rebuild the instruction list (and the parallel span list).
    inserts.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let old = std::mem::take(&mut func.blocks[block_idx].instructions);
    let old_spans = std::mem::take(&mut func.blocks[block_idx].source_spans);
    let has_spans = !old_spans.is_empty();
    let mut new_insts: Vec<Instruction> = Vec::with_capacity(old.len());
    let mut new_spans: Vec<Span> = Vec::new();
    let mut next_insert = 0usize;
    for (i, inst) in old.into_iter().enumerate() {
        while next_insert < inserts.len() && inserts[next_insert].0 == i {
            let (_, _, ins) = inserts[next_insert].clone();
            new_insts.push(ins);
            if has_spans {
                new_spans.push(*old_spans.get(i).unwrap_or(&Span::new(0, 0, 0)));
            }
            next_insert += 1;
        }
        if plan.removed.contains(&i) {
            continue;
        }
        new_insts.push(rewrite_uses(inst, &extract_dest));
        if has_spans {
            // Spans are USUALLY parallel to instructions, but blocks
            // exist where they are not (the backend's own rebuilds tolerate
            // skew); never index blindly (ICE: vec_arx_scalar_spelling_sse2).
            new_spans.push(*old_spans.get(i).unwrap_or(&Span::new(0, 0, 0)));
        }
    }
    // A pack's sched is always < the seed store's slot < n, so any
    // trailing inserts here would be a scheduling bug — append them
    // anyway (defensive: never DROP an instruction).
    while next_insert < inserts.len() {
        let (_, _, ins) = inserts[next_insert].clone();
        new_insts.push(ins);
        if has_spans {
            new_spans.push(Span::new(0, 0, 0));
        }
        next_insert += 1;
    }
    func.blocks[block_idx].instructions = new_insts;
    func.blocks[block_idx].source_spans = new_spans;

    // Terminator use rewrite.
    let term = std::mem::replace(
        &mut func.blocks[block_idx].terminator,
        Terminator::Return(None),
    );
    func.blocks[block_idx].terminator = rewrite_term_uses(term, &extract_dest);

    if debug {
        eprintln!(
            "[BB-SLP] fn={} block B{}: packed {}x{:?} ({} packs, benefit {})",
            func.name,
            block_idx,
            width,
            cand.ty,
            plan.packs.len(),
            plan.benefit
        );
    }
    true
}

/// Rewrite surviving uses of removed lanes to their extracts. Goes
/// through the CANONICAL mutation walkers (`for_each_operand_mut` for
/// Operand-wrapped uses, `for_each_value_use_mut` for the bare-`Value`
/// uses like pointers, memcpy endpoints and intrinsic `dest_ptr`s) — the
/// hand-rolled match this replaced missed Call/Phi/InlineAsm/atomic
/// operand sites and silently DROPPED the intrinsic `dest_ptr` rewrite
/// (`sub(&mut Operand::Value(*p))` mutated a temporary).
fn rewrite_uses(mut inst: Instruction, map: &FxHashMap<u32, Value>) -> Instruction {
    if map.is_empty() {
        return inst;
    }
    inst.for_each_operand_mut(|op| {
        if let Operand::Value(v) = op {
            if let Some(&nv) = map.get(&v.0) {
                *op = Operand::Value(nv);
            }
        }
    });
    inst.for_each_value_use_mut(|v| {
        if let Some(&nv) = map.get(&v.0) {
            *v = nv;
        }
    });
    inst
}

fn rewrite_term_uses(mut term: Terminator, map: &FxHashMap<u32, Value>) -> Terminator {
    if map.is_empty() {
        return term;
    }
    term.for_each_operand_mut(|op| {
        if let Operand::Value(v) = op {
            if let Some(&nv) = map.get(&v.0) {
                *op = Operand::Value(nv);
            }
        }
    });
    term
}
