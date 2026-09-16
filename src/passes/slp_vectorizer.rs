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
use crate::ir::ops::{IrBinOp, IrCmpOp, IrUnaryOp};
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

/// Symbolic affine term `var*mult + off` for a GEP byte-offset operand,
/// evaluated structurally over the SAME expression DAG the machine computes.
///
/// This is the generalization that unlocks indexed straight-line code
/// (`a[i-1..i+2]` windows, stencils, inlined index math). The canonical
/// pre-SLP shapes are produced by `simplify` + the lowering pipeline:
///
/// * `Cast(Add(i, k), I32→I64)` — the frontend's promoted index (and,
///   after cast-absorption in the self-add fold, the raw I32 `Add(i,k)`
///   used directly in an I64 add);
/// * `Add(x, x)` — `simplify` canonicalizes `Mul(x, 2)` into a self-add,
///   and the frontend's `Mul(Cast(i+k), sizeof)` is exactly that;
/// * `Shl(x, k)` — `simplify` canonicalizes `Mul(x, 2^k)` into a shift;
/// * `Mul(x, c)`, `Sub(v, const)` — the remaining scale/subtract forms;
/// * nesting of all of the above (`2*((i+1) + (i+1))`).
///
/// **Soundness argument** (the same one the original `Add(var, Const)`
/// recognition rests on): the term is a symbolic evaluation of the machine's
/// own arithmetic. Every wrap an intermediate could take (I32 add mod 2^32,
/// I64 add mod 2^64) corresponds to an index falling outside the accessed
/// object's bounds — undefined behavior in C, exactly the assumption GCC,
/// Clang, and LLVM's `inbounds` GEPs make. In every defined execution the
/// intermediates do not wrap, the machine address equals
/// `base + var*mult + off`, and two accesses the model calls consecutive
/// really are `size` bytes apart. The sign/zero choice of the implicit
/// widening is likewise immaterial: in a defined program the widened value
/// is the mathematical index either way.
///
/// Terms are always kept canonical: `mult > 0` when `var` is `Some`
/// (negative coefficients — `Sub(const, var)`, `Mul` by a negative —
/// reject the whole GEP instead; they are never array strides), and all
/// arithmetic is checked (a shape whose scale or offset does not fit an
/// i64 was also unwieldy for the old recognizer — it stays opaque).
#[derive(Clone, Copy, Debug)]
struct AffineTerm {
    var: Option<Value>,
    /// Positive when `var` is `Some`; meaningless (0) for constants.
    mult: i64,
    off: i64,
}

/// Maximum structural depth of an offset expression. Real shapes nest at
/// most four levels (Cast → Add/Sub → self-add/Shl/Mul → Cast); the cap only
/// bounds pathological compile time, and a term hitting it degrades to an
/// opaque variable leaf — never unsound.
const AFFINE_DEPTH: usize = 8;

fn ty_is_integer(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::I128
            | IrType::U128
    )
}

/// The opaque variable leaf: `v` itself as the unit-scale index.
fn opaque_var(v: Value) -> AffineTerm {
    AffineTerm {
        var: Some(v),
        mult: 1,
        off: 0,
    }
}

fn affine_term(
    block: &BasicBlock,
    def_pos: &FxHashMap<u32, usize>,
    op: &Operand,
    depth: usize,
) -> Option<AffineTerm> {
    match op {
        Operand::Const(c) => Some(AffineTerm {
            var: None,
            mult: 0,
            off: c.to_i64()?,
        }),
        Operand::Value(v) => {
            let Some(&i) = def_pos.get(&v.0) else {
                // Foreign value (param, other-block IV): an opaque index.
                return Some(opaque_var(*v));
            };
            if depth == 0 {
                return Some(opaque_var(*v));
            }
            match &block.instructions[i] {
                // Widening / same-size integer cast: transparent — the
                // widened value is the same number (see the soundness
                // argument above). NARROWING casts truncate and float
                // casts are not affine: both degrade to the opaque leaf
                // of the cast RESULT (still an SSA value, still exact).
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if ty_is_integer(*from_ty)
                    && ty_is_integer(*to_ty)
                    && to_ty.size() >= from_ty.size() =>
                {
                    affine_term(block, def_pos, src, depth - 1)
                }
                // `x + y`: const+const folds, var+const shifts the offset,
                // and the SAME variable on both sides composes the scale
                // (`Add(x, x)` — the `Mul(x,2)` canonicalization — is the
                // `mult = 2m` special case; `2i + 3i` shapes fold the same
                // way). Two different variables cannot compose: opaque leaf.
                Instruction::BinOp {
                    op: IrBinOp::Add,
                    lhs,
                    rhs,
                    ..
                } => {
                    let l = affine_term(block, def_pos, lhs, depth - 1)?;
                    let r = affine_term(block, def_pos, rhs, depth - 1)?;
                    match (&l.var, &r.var) {
                        (None, None) => Some(AffineTerm {
                            var: None,
                            mult: 0,
                            off: l.off.checked_add(r.off)?,
                        }),
                        (None, Some(_)) | (Some(_), None) => {
                            let (mut t, c) = if l.var.is_none() {
                                (r, l.off)
                            } else {
                                (l, r.off)
                            };
                            t.off = t.off.checked_add(c)?;
                            Some(t)
                        }
                        (Some(lv), Some(rv)) if lv == rv => {
                            let mult = l.mult.checked_add(r.mult)?;
                            if mult == 0 {
                                return Some(AffineTerm {
                                    var: None,
                                    mult: 0,
                                    off: l.off.checked_add(r.off)?,
                                });
                            }
                            Some(AffineTerm {
                                var: l.var,
                                mult,
                                off: l.off.checked_add(r.off)?,
                            })
                        }
                        (Some(_), Some(_)) => Some(opaque_var(*v)),
                    }
                }
                // `v - k`: the `i - k` index form (`a[i-1]`). The
                // coefficient of `v` stays positive.
                Instruction::BinOp {
                    op: IrBinOp::Sub,
                    lhs,
                    rhs,
                    ..
                } => {
                    let mut t = affine_term(block, def_pos, lhs, depth - 1)?;
                    let Operand::Const(c) = rhs else {
                        // var - var or var - const-var: not a plain shift.
                        return Some(opaque_var(*v));
                    };
                    t.off = t.off.checked_sub(c.to_i64()?)?;
                    Some(t)
                }
                // `x * c` (either operand order): scale the whole term.
                // A negative or zero scale cannot keep `mult` positive —
                // the product becomes the opaque leaf.
                Instruction::BinOp {
                    op: IrBinOp::Mul,
                    lhs,
                    rhs,
                    ..
                } => {
                    let (x, c) = match (lhs, rhs) {
                        (x, Operand::Const(c)) => (x, c.to_i64()?),
                        (Operand::Const(c), x) => (x, c.to_i64()?),
                        _ => return Some(opaque_var(*v)),
                    };
                    let t = affine_term(block, def_pos, x, depth - 1)?;
                    scale_term(t, c, *v)
                }
                // `x << k` == `x * 2^k` in wrapping two's complement —
                // exact at every wrap width, so the affine scale is exact.
                Instruction::BinOp {
                    op: IrBinOp::Shl,
                    lhs,
                    rhs,
                    ..
                } => {
                    let Operand::Const(c) = rhs else {
                        return Some(opaque_var(*v));
                    };
                    let k = c.to_i64()?;
                    if !(0..=62).contains(&k) {
                        return Some(opaque_var(*v));
                    }
                    let t = affine_term(block, def_pos, lhs, depth - 1)?;
                    scale_term(t, 1i64.checked_shl(k as u32)?, *v)
                }
                _ => Some(opaque_var(*v)),
            }
        }
    }
}

/// Scale `t` by `s` (`s != 0`, `mult` stays positive) or degrade to the
/// opaque leaf of `v`.
fn scale_term(t: AffineTerm, s: i64, v: Value) -> Option<AffineTerm> {
    if s == 0 {
        return Some(AffineTerm {
            var: None,
            mult: 0,
            off: 0,
        });
    }
    match t.var {
        None => Some(AffineTerm {
            var: None,
            mult: 0,
            off: t.off.checked_mul(s)?,
        }),
        Some(var) => {
            let mult = t.mult.checked_mul(s)?;
            if mult <= 0 {
                return Some(opaque_var(v));
            }
            Some(AffineTerm {
                var: Some(var),
                mult,
                off: t.off.checked_mul(s)?,
            })
        }
    }
}

/// Evaluate a pointer value to a symbolic address. Recognizes raw pointer
/// values, `GEP(base, Const)` chains, and — through `affine_term` — every
/// scaled/indexed offset shape the canonicalizer emits (`Add(x,x)`
/// doubling, `Shl`, `Mul`, `Sub`, widening `Cast`s, and their nesting).
fn eval_sym_addr(
    block: &BasicBlock,
    def_pos: &FxHashMap<u32, usize>,
    ptr: Value,
) -> Option<SymAddr> {
    eval_sym_addr_d(block, def_pos, ptr, 4)
}

fn eval_sym_addr_d(
    block: &BasicBlock,
    def_pos: &FxHashMap<u32, usize>,
    ptr: Value,
    depth: usize,
) -> Option<SymAddr> {
    // A pointer defined OUTSIDE this block (entry-block ParamRef or
    // GlobalAddr, a foreign block's computation) is an opaque base:
    // affine-equal to itself with no local offset structure. Returning
    // None on the lookup miss instead silently dropped every offset-0
    // store whose pointer is the raw global/param value — the frontend's
    // spelling for `global[0]` — so multi-block functions (anything with
    // a loop) never seeded those streams (D9).
    let inst = match def_pos.get(&ptr.0).and_then(|&i| block.instructions.get(i)) {
        Some(inst) => inst,
        None => {
            return Some(SymAddr {
                base: ptr,
                var: None,
                mult: 1,
                off: 0,
            });
        }
    };
    match inst {
        Instruction::GetElementPtr { base, offset, .. } => {
            // Base of a GEP may itself be a const-offset GEP — fold
            // recursively; a base that carries its own index variable
            // degrades to the inner GEP's value as an opaque base (two
            // accesses through the SAME base-GEP value still stream
            // together — the `p = &a[i]; p[0..3]` shape).
            let mut addr = match eval_sym_addr_d(block, def_pos, *base, depth.saturating_sub(1)) {
                Some(a) if a.var.is_none() => a,
                _ => SymAddr {
                    base: *base,
                    var: None,
                    mult: 1,
                    off: 0,
                },
            };
            // The offset term: a constant folds into `off`; a variable
            // term (with its scale and constant part from `affine_term`)
            // becomes the stream's index. `addr` never carries a var at
            // this point (var-carrying bases degraded above), so at most
            // one index variable composes here.
            let t = affine_term(block, def_pos, offset, AFFINE_DEPTH)?;
            match t.var {
                None => {
                    addr.off = addr.off.checked_add(t.off)?;
                }
                Some(v) => {
                    if addr.var.is_some() {
                        // Two different index variables in one address
                        // (`a[i][j]`): no single-stream model — skip.
                        return None;
                    }
                    addr.var = Some(v);
                    addr.mult = t.mult as u64;
                    addr.off = addr.off.checked_add(t.off)?;
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
            extract: Some(IntrinsicOp::VecExtractLaneF64x4),
            size: 8,
        }),
        (IrType::F32, 4) => Some(VecFamily {
            load: IntrinsicOp::VecLoadF32x4,
            store: IntrinsicOp::VecStoreF32x4,
            broadcast: IntrinsicOp::VecBroadcastF32x4,
            zero: IntrinsicOp::VecZeroF32x4,
            pack2: None,
            pack4: None,
            extract: Some(IntrinsicOp::VecExtractLaneF32x4),
            size: 4,
        }),
        (IrType::F32, 8) if avx2 => Some(VecFamily {
            load: IntrinsicOp::VecLoadF32x8,
            store: IntrinsicOp::VecStoreF32x8,
            broadcast: IntrinsicOp::VecBroadcastF32x8,
            zero: IntrinsicOp::VecZeroF32x8,
            pack2: None,
            pack4: None,
            extract: Some(IntrinsicOp::VecExtractLaneF32x8),
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
            extract: Some(IntrinsicOp::VecExtractLaneI32x8),
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
            extract: Some(IntrinsicOp::VecExtractLaneI64x4),
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
            extract: Some(IntrinsicOp::VecExtractLaneI16x8),
            size: 2,
        }),
        (IrType::I16 | IrType::U16, 16) if avx2 => Some(VecFamily {
            load: IntrinsicOp::VecLoadI16x16,
            store: IntrinsicOp::VecStoreI16x16,
            broadcast: IntrinsicOp::VecBroadcastI16x16,
            // 256-bit zero: no VecZeroI16x16 — the broadcast of a zero
            // constant (movd+vpbroadcastw; the all-ones splat folds to
            // one vpcmpeqd at emission).
            zero: IntrinsicOp::VecBroadcastI16x16,
            pack2: None,
            pack4: None,
            extract: Some(IntrinsicOp::VecExtractLaneI16x16),
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
        (IrType::I8 | IrType::U8, 32) if avx2 => Some(VecFamily {
            load: IntrinsicOp::VecLoadI8x32,
            store: IntrinsicOp::VecStoreI8x32,
            broadcast: IntrinsicOp::VecBroadcastI8x32,
            zero: IntrinsicOp::VecBroadcastI8x32,
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
        (IrType::I16 | IrType::U16, 16) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI16x16),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI16x16),
            IrBinOp::Mul => Some(IntrinsicOp::VecMulI16x16),
            IrBinOp::And => Some(IntrinsicOp::VecAndI16x16),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI16x16),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI16x16),
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
        (IrType::I8 | IrType::U8, 32) => match op {
            IrBinOp::Add => Some(IntrinsicOp::VecAddI8x32),
            IrBinOp::Sub => Some(IntrinsicOp::VecSubI8x32),
            IrBinOp::And => Some(IntrinsicOp::VecAndI8x32),
            IrBinOp::Or => Some(IntrinsicOp::VecOrI8x32),
            IrBinOp::Xor => Some(IntrinsicOp::VecXorI8x32),
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

/// Map a shift lane op to its packed intrinsic — the uniform CONSTANT
/// amount forms only (`psllw/psrlw/psraw/pslld/psrld/psrad/psllq/psrlq`
/// and their VEX counterparts). None where the ISA has no packed form:
/// byte lanes (no packed byte shift before AVX-512) and I64 arithmetic
/// right shift (no packed qword `psraq` before AVX-512). Those lanes
/// degrade to gathers where a family has one, else reject the seed.
fn packed_shift(op: IrBinOp, ty: IrType, width: usize) -> Option<IntrinsicOp> {
    match (ty, width) {
        (IrType::I16 | IrType::U16, 16) => match op {
            IrBinOp::Shl => Some(IntrinsicOp::VecShlI16x16),
            IrBinOp::LShr => Some(IntrinsicOp::VecLShrI16x16),
            IrBinOp::AShr => Some(IntrinsicOp::VecAShrI16x16),
            _ => None,
        },
        (IrType::I16 | IrType::U16, 8) => match op {
            IrBinOp::Shl => Some(IntrinsicOp::VecShlI16x8),
            IrBinOp::LShr => Some(IntrinsicOp::VecLShrI16x8),
            IrBinOp::AShr => Some(IntrinsicOp::VecAShrI16x8),
            _ => None,
        },
        (IrType::I32 | IrType::U32, 8) => match op {
            IrBinOp::Shl => Some(IntrinsicOp::VecShlI32x8),
            IrBinOp::LShr => Some(IntrinsicOp::VecLShrI32x8),
            IrBinOp::AShr => Some(IntrinsicOp::VecAShrI32x8),
            _ => None,
        },
        (IrType::I32 | IrType::U32, 4) => match op {
            IrBinOp::Shl => Some(IntrinsicOp::VecShlI32x4),
            IrBinOp::LShr => Some(IntrinsicOp::VecLShrI32x4),
            IrBinOp::AShr => Some(IntrinsicOp::VecAShrI32x4),
            _ => None,
        },
        (IrType::I64 | IrType::U64, 4) => match op {
            IrBinOp::Shl => Some(IntrinsicOp::VecShlI64x4),
            IrBinOp::LShr => Some(IntrinsicOp::VecLShrI64x4),
            _ => None,
        },
        (IrType::I64 | IrType::U64, 2) => match op {
            IrBinOp::Shl => Some(IntrinsicOp::VecShlI64x2),
            IrBinOp::LShr => Some(IntrinsicOp::VecLShrI64x2),
            _ => None,
        },
        _ => None,
    }
}

/// The all-ones constant of a lane type (`-1` in every width — the bit
/// pattern `try_all_ones_splat` lowers to one `pcmpeqd`/`vpcmpeqd`).
fn all_ones_const(ty: IrType) -> Option<IrConst> {
    match ty {
        IrType::I8 | IrType::U8 => Some(IrConst::I8(-1)),
        IrType::I16 | IrType::U16 => Some(IrConst::I16(-1)),
        IrType::I32 | IrType::U32 => Some(IrConst::I32(-1)),
        IrType::I64 | IrType::U64 => Some(IrConst::I64(-1)),
        _ => None,
    }
}

/// Map an FP lane type/width to its packed strict min/max intrinsic.
/// FP-only: integer min/max needs no such fold (cmp+blendv is the exact
/// integer lowering, and the SSE2 baseline lacks dword pminsd/pmaxsd).
fn packed_minmax(is_max: bool, ty: IrType, width: usize) -> Option<IntrinsicOp> {
    match (ty, width) {
        (IrType::F32, 8) => Some(if is_max {
            IntrinsicOp::VecMaxF32x8
        } else {
            IntrinsicOp::VecMinF32x8
        }),
        (IrType::F32, 4) => Some(if is_max {
            IntrinsicOp::VecMaxF32x4
        } else {
            IntrinsicOp::VecMinF32x4
        }),
        (IrType::F64, 4) => Some(if is_max {
            IntrinsicOp::VecMaxF64x4
        } else {
            IntrinsicOp::VecMinF64x4
        }),
        (IrType::F64, 2) => Some(if is_max {
            IntrinsicOp::VecMaxF64x2
        } else {
            IntrinsicOp::VecMinF64x2
        }),
        _ => None,
    }
}

/// Strip identity casts (`Cast{ty→ty}` — the frontend's re-spelling
/// wrappers; from_ty == to_ty is the identity function by definition).
fn strip_identity_casts(ctx: &BlockCtx, o: &Operand) -> Operand {
    let block = ctx.block;
    let mut cur = o.clone();
    for _ in 0..8 {
        let Operand::Value(v) = &cur else { break };
        let Some(&i) = ctx.def_pos.get(&v.0) else {
            break;
        };
        match &block.instructions[i] {
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if from_ty == to_ty => {
                cur = src.clone();
            }
            _ => break,
        }
    }
    cur
}

/// Two operands name the same SOURCE when (after identity-cast stripping)
/// they are the same value, or two loads of the same symbolic address
/// with NO memory write between them — the pre-CSE frontend emits one
/// load per spelling side, and the interval check is the whole
/// same-value proof (any write between the two loads could change the
/// observed bytes; rule (c) covers the pack's own lane range only).
fn same_source(ctx: &BlockCtx, a: &Operand, b: &Operand) -> bool {
    let a = strip_identity_casts(ctx, a);
    let b = strip_identity_casts(ctx, b);
    if a == b {
        return true;
    }
    let block = ctx.block;
    let addr_of = |o: &Operand| -> Option<(SymAddr, usize)> {
        let Operand::Value(v) = o else { return None };
        let &i = ctx.def_pos.get(&v.0)?;
        match &block.instructions[i] {
            Instruction::Load {
                ptr,
                seg_override,
                volatile,
                ..
            } if *seg_override == AddressSpace::Default && !*volatile => {
                eval_sym_addr(block, &ctx.def_pos, *ptr).map(|a| (a, i))
            }
            _ => None,
        }
    };
    match (addr_of(&a), addr_of(&b)) {
        (Some((aa, pa)), Some((ab, pb))) => {
            if aa != ab {
                return false;
            }
            let (plo, phi) = if pa < pb { (pa, pb) } else { (pb, pa) };
            (plo + 1..phi).all(|q| !is_memory_write(&block.instructions[q]))
        }
        _ => false,
    }
}

/// Resolve an operand to a compile-time i64, looking through the
/// `Copy`/`Cast` constant materializations the frontend emits before
/// `simplify` folds them into inline constants (the early SLP sweep runs
/// first: `x << 7` arrives as `Shl(x, Copy(Cast(Const(7))))`).
///
/// Exactness: a `Copy` forwards its value; an integer `Cast` is followed
/// only when the constant fits the cast's OUTPUT width — the machine
/// value at the use site is then the same number (a narrowing cast of an
/// out-of-range constant would truncate and is refused). Callers bound
/// the result to the lane's defined shift domain on top.
fn const_amount(ctx: &BlockCtx, op: &Operand) -> Option<i64> {
    let mut cur = op.clone();
    for _ in 0..8 {
        match &cur {
            Operand::Const(c) => return c.to_i64(),
            Operand::Value(v) => {
                let Some(&i) = ctx.def_pos.get(&v.0) else {
                    return None;
                };
                match &ctx.block.instructions[i] {
                    Instruction::Copy { src, .. } => {
                        cur = src.clone();
                    }
                    Instruction::Cast { src, to_ty, .. } => {
                        let Operand::Const(c) = src else {
                            match src {
                                Operand::Value(nv) => {
                                    cur = Operand::Value(*nv);
                                    continue;
                                }
                                _ => return None,
                            }
                        };
                        let val = c.to_i64()?;
                        let bits = to_ty.size() as i64 * 8;
                        if !ty_is_integer(*to_ty) || bits >= 64 {
                            return Some(val);
                        }
                        let lim = 1i64 << (bits - 1);
                        if (-lim..lim).contains(&val) {
                            return Some(val);
                        }
                        return None;
                    }
                    _ => return None,
                }
            }
        }
    }
    None
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
    /// Packed lane shift by a uniform CONSTANT amount:
    /// `dest = vec_op(val, amount)`. Two shapes share the kind:
    /// plain scalar shift lanes (lane_vals = the shift defs, removed by
    /// the rewrite) and the synthetic halves of a rotate decomposition
    /// (lane_vals empty — scheduled by the leaf fixpoint like a splat,
    /// cost 1 each).
    ShiftImm {
        vec_op: IntrinsicOp,
        val: usize,
        amount: i64,
    },
    /// FP strict min/max fold: `dest = MINPS/MAXPS(src1, src2)` with the
    /// x86 operand contract (the SECOND source is returned on unordered
    /// and both-zero lanes). `lane_vals` are the folded Select defs;
    /// `cond_lanes` the Cmp defs that die with them (removed, and
    /// external-use-checked, but never extracted — a scalar bool cannot
    /// be reconstructed from the packed mask cheaply).
    ///
    /// Lane-exactness (the same proof the loop vectorizer's
    /// `MapExpr::MinMax` carries): a strict-ordered C ternary
    /// `l < r ? l : r` returns the false arm exactly when MINPS returns
    /// src2 — every special case (NaN, ±0 of either sign, equal values)
    /// agrees — so the four strict spellings fold with the false arm in
    /// the src2 position. Non-strict `<=`/`>=` forms differ on ±0 and
    /// keep the scalar lowering.
    FpMinMax {
        vec_op: IntrinsicOp,
        lhs: usize,
        rhs: usize,
        cond_lanes: Vec<Value>,
    },
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
    // Shared cross-block-use map (invariant across this pass's rewrites —
    // see `cross_block_use_map`); one O(function) pass instead of a
    // rescan per fired seed.
    let cross = cross_block_use_map(func);
    // Fixpoint: each fired seed rewrites its block, so re-scan afterwards.
    for _round in 0..24 {
        let mut fired = 0usize;
        for b in 0..func.blocks.len() {
            while slp_block_once(func, b, &cross, debug) == 1 {
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

/// Values (function-wide) that have at least one use OUTSIDE their
/// defining block — the shared authority for rule (b).
///
/// Computed ONCE per `run_bb_slp` invocation: the set is invariant across
/// every rewrite this pass performs. A rewrite of block b only (1) removes
/// IN-BLOCK uses of lanes (their defs are in b by construction) and
/// (2) adds brand-new values (vector dests, extracts) used only within b;
/// a rewrite of block c replaces uses of c-DEFINED lanes with extracts —
/// it never touches uses of values defined elsewhere. Hence no value's
/// use-block set ever gains or loses a FOREIGN-block use after this pass
/// starts, and the entry-time answer stays exact for every later
/// query.
///
/// (Before this map, `build_ctx` rescanned every other block per fired
/// seed — O(function) per seed, O(seeds × function) per block, a
/// measurable compile-time tax on large functions.)
fn cross_block_use_map(func: &IrFunction) -> FxHashSet<u32> {
    let mut def_block: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                def_block.insert(d.0, bi);
            }
        }
    }
    let mut cross: FxHashSet<u32> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            inst.for_each_used_value(|vid| {
                if def_block.get(&vid).is_some_and(|&db| db != bi) {
                    cross.insert(vid);
                }
            });
        }
        block.terminator.for_each_used_value(|vid| {
            if def_block.get(&vid).is_some_and(|&db| db != bi) {
                cross.insert(vid);
            }
        });
    }
    cross
}

struct BlockCtx<'a> {
    block: &'a BasicBlock,
    def_pos: FxHashMap<u32, usize>,
    /// Use positions (instruction indices; `usize::MAX` = terminator) of
    /// each value IN THIS BLOCK.
    uses: FxHashMap<u32, Vec<usize>>,
    /// Values with uses outside their DEFINING block (the shared
    /// cross-block map, see `cross_block_use_map`). For a lane defined
    /// in this block this is exactly "used in some other block": a
    /// packed lane in the set cannot be replaced by a block-local
    /// extract — reject its seed.
    external_uses: &'a FxHashSet<u32>,
}

fn build_ctx<'a>(
    func: &'a IrFunction,
    block_idx: usize,
    cross: &'a FxHashSet<u32>,
) -> BlockCtx<'a> {
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
    BlockCtx {
        block,
        def_pos,
        uses,
        external_uses: cross,
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
/// length. Family-aware: every lane type now has both a 128-bit and a
/// 256-bit family under AVX2 (the byte runs take I8x32, halfword runs
/// I16x16), so the 32-byte runs pack at full width instead of falling
/// back (the original `preferred_width` returned 32 lanes for every
/// ≥32-byte run and `family_for` then rejected the candidate — those
/// runs never vectorized at all until the family-aware fallback).
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
    /// Named global objects: value id -> object name. Distinct names are
    /// distinct objects (C11 object identity): the linker never merges
    /// differently-named data objects, so their storage is provably
    /// disjoint — the assumption GCC and Clang make for every pair of
    /// file-scope objects. (Two GlobalAddr values for the SAME name are
    /// the same object and stay "may alias".)
    globals: FxHashMap<u32, String>,
    /// Fresh frame objects (allocas): an alloca is created after entry,
    /// so no pre-existing value can designate its storage; distinct
    /// allocas are distinct objects.
    allocas: FxHashSet<u32>,
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
                    Instruction::GlobalAddr { dest, name } => {
                        rb.independent.insert(dest.0);
                        rb.globals.insert(dest.0, name.clone());
                    }
                    Instruction::Alloca { dest, .. } => {
                        rb.independent.insert(dest.0);
                        rb.allocas.insert(dest.0);
                    }
                    _ => {}
                }
            }
        }
        rb
    }

    /// May the two base values be assumed to designate disjoint objects?
    /// Three provable-disjointness classes:
    /// 1. C11 6.7.3.1 `restrict` contracts (the original rule: one base
    ///    carries restrict, the other is provably not based on it).
    /// 2. Object identity for NAMED GLOBALS: two distinct names are two
    ///    distinct objects with disjoint storage (the linker never merges
    ///    differently-named data objects — mergeable string-literal
    ///    sections are not named-object `.data`/`.bss`, and ICF folds
    ///    only code).
    /// 3. Object identity for FRESH FRAME OBJECTS: an alloca's storage
    ///    comes into existence at entry, so no global or parameter can
    ///    designate it, and two distinct allocas never overlap.
    /// A global and a NON-restrict parameter may still alias (the caller
    /// can pass &global); two parameters may alias; a restrict parameter
    /// and its own derived values are excluded by rule 1's independence
    /// requirement. This extension is what lets the common two-global
    /// shape (`pos[i] = k * vel[i]` with file-scope `pos`/`vel` arrays)
    /// pass rule (e) — previously every cross-global seed was rejected
    /// as "may alias".
    fn disjoint(&self, a: u32, b: u32) -> bool {
        if a == b {
            return false;
        }
        if (self.noalias.contains(&a) && self.independent.contains(&b))
            || (self.noalias.contains(&b) && self.independent.contains(&a))
        {
            return true;
        }
        // Distinct named globals.
        if let (Some(ga), Some(gb)) = (self.globals.get(&a), self.globals.get(&b)) {
            return ga != gb;
        }
        // A named global and a fresh frame object never overlap.
        if (self.globals.contains_key(&a) && self.allocas.contains(&b))
            || (self.globals.contains_key(&b) && self.allocas.contains(&a))
        {
            return true;
        }
        // Two distinct fresh frame objects never overlap.
        self.allocas.contains(&a) && self.allocas.contains(&b)
    }
}

fn slp_block_once(
    func: &mut IrFunction,
    block_idx: usize,
    cross: &FxHashSet<u32>,
    debug: bool,
) -> usize {
    // All analysis under one immutable borrow; the plans are fully owned
    // (no lifetimes into the block), so the mutable rewrite afterwards
    // is borrow-clean.
    let (candidates, plans): (Vec<SeedCandidate>, Vec<Option<Plan>>) = {
        let ctx = build_ctx(func, block_idx, cross);
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

    // 1. Splat: all lanes the same operand — BIT-EXACTLY. `Operand`'s
    // derived equality compares `IrConst::F32/F64` as IEEE floats, under
    // which `-0.0 == +0.0`; a mask like {-0.0, +0.0, -0.0, ...} would
    // pass as a "splat" of +0.0, take the zero-vector path, and destroy
    // the sign bits (simd_blendv256: the blend mask became all +0.0 and
    // every lane selected operand a). `LaneKey` hashes the raw bits —
    // exactly the predicate the dedup map already uses.
    if lanes.windows(2).all(|w| lane_key(&w[0]) == lane_key(&w[1])) {
        let src = lanes[0].clone();
        // Bit-exact zero test (`IrConst::is_all_zero_bits`): +0.0 is the
        // all-zero bit pattern (exactly what the `VecZero*` lowerings
        // produce), while `-0.0` has its sign bit set and must NOT take
        // the zero path. `to_i64() == Some(0)` — the previous predicate —
        // returns `None` for every float constant, so ALL FP constant
        // splats (including `q[i] = 0.0` stores) stayed scalar.
        let is_zero = match &src {
            Operand::Const(c) => c.is_all_zero_bits(),
            _ => false,
        };
        // FP constant splats ARE audited-sound at the backend: every FP
        // broadcast arm stages through `emit_fp_operand_to_xmm`, whose
        // Const arm materializes the exact bit pattern via the FP
        // constant pool (`movsd label(%rip)`) or `xorpd` for +0.0. The
        // old rejection here was based on the GPR-staging assumption
        // that only ever applied to the INTEGER broadcast arms.
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
        //
        // NESTED promoted trees (`q[i]*m[i]*k` — the intermediates stay
        // I32 all the way to the store's trunc) demote level by level: a
        // lane of this pack may itself be a same-op I32 binop whose
        // operands are widening casts, fitting constants, or further
        // nested I32 binops (passed through so the recursion demotes
        // them). Exactness under nesting is modular arithmetic: for every
        // op in the promotable set, low_n(A OP B) == low_n(low_n(A) OP_n
        // low_n(B)) — two's-complement wrap is mod-2^32 and the low n
        // bits commute with each level's mod-2^n reduction — so the
        // STORED sub-word value is bit-identical whether the tree
        // computes in I32 (with or without intermediate wrap, signed
        // overflow being UB in C anyway) or at the lane width.
        if matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
            // Is `t` an integer type strictly wider than the lane?
            let wider_int = |t: IrType| ty_is_integer(t) && t.size() > ty.size();
            // Lane defs: truncating casts (the root of a demoted tree),
            // or direct promoted-width binops (a nested level reached
            // through the recursion below). The promoted width is ANY
            // wider integer type: SIGNED promotion lands in I32, but the
            // UNSIGNED spelling promotes to U32 — and explicit casts can
            // build I16/U16 intermediates over byte lanes. The demotion
            // is exact at every wider width (mod-2^lane arithmetic
            // commutes with And/Or/Xor/Add/Sub/Mul).
            let is_promotable_wide_binop = |v: &Value| {
                matches!(
                    ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]),
                    Some(Instruction::BinOp { op, ty: bty, .. })
                        if wider_int(*bty)
                            && matches!(
                                op,
                                IrBinOp::Add
                                    | IrBinOp::Sub
                                    | IrBinOp::Mul
                                    | IrBinOp::And
                                    | IrBinOp::Or
                                    | IrBinOp::Xor
                                    | IrBinOp::Shl
                                    | IrBinOp::LShr
                                    | IrBinOp::AShr
                            )
                )
            };
            let lanes_ok = vals.iter().all(|v| {
                matches!(
                    ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]),
                    Some(Instruction::Cast { from_ty, to_ty, .. })
                        if *to_ty == ty && wider_int(*from_ty)
                ) || is_promotable_wide_binop(v)
            });
            if lanes_ok {
                // The computing sources: the promoted-width binop
                // feeding each truncation, or the nested binop lane
                // itself.
                let mut promoted: Vec<Value> = Vec::with_capacity(width);
                let mut ok = true;
                for v in &vals {
                    let i = ctx.def_pos[&v.0];
                    let sv = match &block.instructions[i] {
                        Instruction::Cast { src, .. } => {
                            let Operand::Value(sv) = src else {
                                ok = false;
                                break;
                            };
                            *sv
                        }
                        _ => *v, // nested binop lane computes itself
                    };
                    if is_promotable_wide_binop(&sv) {
                        promoted.push(sv);
                    } else {
                        ok = false;
                        break;
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
                                // the lane type (the zext/sext C promotion —
                                // into ANY wider integer width), or a nested
                                // same-set promoted binop — passed through
                                // unchanged so the recursion demotes it one
                                // level deeper.
                                Operand::Value(w) => {
                                    match ctx.def_pos.get(&w.0).map(|&k| &block.instructions[k]) {
                                        Some(Instruction::Cast {
                                            src,
                                            from_ty,
                                            to_ty,
                                            ..
                                        }) if *from_ty == ty && wider_int(*to_ty) => {
                                            Some(src.clone())
                                        }
                                        _ if is_promotable_wide_binop(w) => {
                                            Some(Operand::Value(*w))
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
                        // Shift lanes: the sub-word semantics depend on the
                        // PROMOTION kind, which the IR encodes in the
                        // widening cast's from_ty — the lane type itself:
                        //   - Shl: the low n bits of a left shift depend
                        //     only on the low n input bits — sound under
                        //     both zext and sext;
                        //   - AShr: sext → psraw; zext (unsigned lanes) →
                        //     the promoted value is non-negative, so the
                        //     arithmetic shift IS logical → psrlw;
                        //   - LShr: zext → psrlw; sext REJECTS (the sign
                        //     bits enter the truncated window:
                        //     trunc(sext(x) >>u k) is no sub-word shift).
                        // The amount must be a compile-time constant,
                        // uniform across lanes, and in [1, lane_bits):
                        // at or above the lane width the hardware masks
                        // the packed count while the promoted shift's low
                        // bits are all zero — never equal.
                        if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                            let lane_bits = ty.size() as i64 * 8;
                            let effective: Option<IrBinOp> = match op {
                                IrBinOp::Shl => Some(IrBinOp::Shl),
                                IrBinOp::AShr => {
                                    if ty.is_signed() {
                                        Some(IrBinOp::AShr)
                                    } else {
                                        Some(IrBinOp::LShr)
                                    }
                                }
                                _ => {
                                    if ty.is_signed() {
                                        None
                                    } else {
                                        Some(IrBinOp::LShr)
                                    }
                                }
                            };
                            let mut amount: Option<i64> = None;
                            let mut amounts_ok = true;
                            for b in &op_b {
                                let Operand::Const(c) = b else {
                                    amounts_ok = false;
                                    break;
                                };
                                let Some(k) = c.to_i64() else {
                                    amounts_ok = false;
                                    break;
                                };
                                if !(1..lane_bits).contains(&k) {
                                    amounts_ok = false;
                                    break;
                                }
                                match amount {
                                    None => amount = Some(k),
                                    Some(a) if a == k => {}
                                    _ => {
                                        amounts_ok = false;
                                        break;
                                    }
                                }
                            }
                            if let (Some(eff), Some(k)) = (effective, amount) {
                                if amounts_ok {
                                    if let Some(vec_op) = packed_shift(eff, ty, width) {
                                        if let Some(val) = build_pack(
                                            ctx,
                                            &op_a,
                                            ty,
                                            width,
                                            fam,
                                            packs,
                                            dedup,
                                            depth + 1,
                                        ) {
                                            let idx = packs.len();
                                            packs.push(Pack {
                                                kind: PackKind::ShiftImm {
                                                    vec_op,
                                                    val,
                                                    amount: k,
                                                },
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
                                    // casts (or, for a nested level, the
                                    // demoted I32 binop itself); the
                                    // widening casts and surviving promoted
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

        // 2b. Unary Not / Neg lanes (integer, at the store's lane width):
        //     composites over the existing pack kinds, both bit-exact in
        //     two's-complement lane arithmetic for EVERY value including
        //     the wrap edges:
        //       Not(x) == Xor(x, -1)            (definitional)
        //       Neg(x) == Sub(0, x)             (two's complement)
        //     The all-ones splat materializes as ONE pcmpeqd/vpcmpeqd
        //     (`try_all_ones_splat`), the zero splat as one vpxor/pxor —
        //     so a 4-lane Not costs 2 vector ops and a 4-lane Neg 2
        //     where the scalar code paid 4 (GCC emits the same shapes).
        //     A unary op at a DIFFERENT type than the seed's lane type
        //     (the sub-word promotion spelling) is left to v2.
        if matches!(
            ty,
            IrType::I8
                | IrType::U8
                | IrType::I16
                | IrType::U16
                | IrType::I32
                | IrType::U32
                | IrType::I64
                | IrType::U64
        ) {
            let unary_lanes_ok = |uop: IrUnaryOp| {
                vals.iter().all(|v| {
                    matches!(
                        ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]),
                        Some(Instruction::UnaryOp {
                            op: lop,
                            ty: uty,
                            ..
                        }) if *lop == uop && *uty == ty
                    )
                })
            };
            if unary_lanes_ok(IrUnaryOp::Not) {
                if packed_binop(IrBinOp::Xor, ty, width).is_some() {
                    let srcs: Vec<Operand> = vals
                        .iter()
                        .map(|v| {
                            let i = ctx.def_pos[&v.0];
                            match &block.instructions[i] {
                                Instruction::UnaryOp { src, .. } => src.clone(),
                                _ => unreachable!(),
                            }
                        })
                        .collect();
                    let ones = all_ones_const(ty).unwrap();
                    if let Some(lhs) =
                        build_pack(ctx, &srcs, ty, width, fam, packs, dedup, depth + 1)
                    {
                        let ones_idx = packs.len();
                        packs.push(Pack {
                            kind: PackKind::Splat {
                                src: Operand::Const(ones),
                                is_zero: false,
                            },
                            lane_vals: Vec::new(),
                            sched: usize::MAX,
                            order: 0,
                        });
                        let idx = packs.len();
                        packs.push(Pack {
                            kind: PackKind::BinOp {
                                op: IrBinOp::Xor,
                                ty,
                                vec_op: packed_binop(IrBinOp::Xor, ty, width).unwrap(),
                                lhs,
                                rhs: ones_idx,
                            },
                            lane_vals: vals.clone(),
                            sched: usize::MAX,
                            order: 0,
                        });
                        dedup.insert(key, idx);
                        return Some(idx);
                    }
                }
            } else if unary_lanes_ok(IrUnaryOp::Neg) {
                if packed_binop(IrBinOp::Sub, ty, width).is_some() {
                    let srcs: Vec<Operand> = vals
                        .iter()
                        .map(|v| {
                            let i = ctx.def_pos[&v.0];
                            match &block.instructions[i] {
                                Instruction::UnaryOp { src, .. } => src.clone(),
                                _ => unreachable!(),
                            }
                        })
                        .collect();
                    if let Some(rhs) =
                        build_pack(ctx, &srcs, ty, width, fam, packs, dedup, depth + 1)
                    {
                        let zero_idx = packs.len();
                        packs.push(Pack {
                            kind: PackKind::Splat {
                                src: Operand::Const(IrConst::Zero),
                                is_zero: true,
                            },
                            lane_vals: Vec::new(),
                            sched: usize::MAX,
                            order: 0,
                        });
                        let idx = packs.len();
                        packs.push(Pack {
                            kind: PackKind::BinOp {
                                op: IrBinOp::Sub,
                                ty,
                                vec_op: packed_binop(IrBinOp::Sub, ty, width).unwrap(),
                                lhs: zero_idx,
                                rhs,
                            },
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

        // 2c. FP strict min/max Select lanes: every lane is
        //     `Select(cond = Cmp(op, l, r), t, f)` in one of the four
        //     STRICT-ORDERED foldable spellings (see PackKind::FpMinMax).
        //     Operand identity is compared BIT-EXACTLY (`lane_key`), never
        //     by `Operand`'s derived float equality — a {-0.0, +0.0}
        //     mismatch under `==` would fold a ternary whose ±0 lanes the
        //     packed op answers differently. Non-strict (<=, >=) compares
        //     and mixed per-lane shapes reject the lanes (v2: per-lane
        //     routing).
        if matches!(ty, IrType::F32 | IrType::F64) {
            let sel_shape =
                |v: &Value| -> Option<(IrCmpOp, Operand, Operand, Operand, Operand, Value)> {
                    let i = ctx.def_pos.get(&v.0)?;
                    let Instruction::Select {
                        cond,
                        true_val,
                        false_val,
                        ..
                    } = &block.instructions[*i]
                    else {
                        return None;
                    };
                    let Operand::Value(cv) = cond else {
                        return None;
                    };
                    let ci = ctx.def_pos.get(&cv.0)?;
                    let Instruction::Cmp {
                        op,
                        lhs,
                        rhs,
                        ty: cty,
                        ..
                    } = &block.instructions[*ci]
                    else {
                        return None;
                    };
                    if *cty != ty {
                        return None;
                    }
                    Some((
                        *op,
                        lhs.clone(),
                        rhs.clone(),
                        true_val.clone(),
                        false_val.clone(),
                        *cv,
                    ))
                };
            if vals.iter().all(|v| sel_shape(v).is_some()) {
                // Uniform fold decision across lanes.
                #[derive(Clone, Copy, PartialEq)]
                enum Fold {
                    Min,
                    Max,
                }
                let mut fold: Option<(Fold, bool)> = None; // (kind, arms_swapped)
                let mut shapes_ok = true;
                for v in &vals {
                    let Some((op, l, r, t, f, _)) = sel_shape(v) else {
                        unreachable!()
                    };
                    let this = match op {
                        IrCmpOp::Slt => {
                            if same_source(ctx, &t, &l) && same_source(ctx, &f, &r) {
                                Some((Fold::Min, false))
                            } else if same_source(ctx, &t, &r) && same_source(ctx, &f, &l) {
                                Some((Fold::Max, true))
                            } else {
                                None
                            }
                        }
                        IrCmpOp::Sgt => {
                            if same_source(ctx, &t, &l) && same_source(ctx, &f, &r) {
                                Some((Fold::Max, false))
                            } else if same_source(ctx, &t, &r) && same_source(ctx, &f, &l) {
                                Some((Fold::Min, true))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    match (this, fold) {
                        (Some(x), None) => fold = Some(x),
                        (Some(x), Some(y)) if x == y => {}
                        _ => {
                            shapes_ok = false;
                            break;
                        }
                    }
                }
                let _dbg_trace = std::env::var("LCCC_DEBUG_SLP_TRACE").is_ok();
                if _dbg_trace {
                    eprintln!("[SLP-TRACE] 2c: sel_shapes ok, fold={}", fold.is_some());
                }
                if shapes_ok && fold.is_some() {
                    let (kind, arms_swapped) = fold.unwrap();
                    let vec_op = packed_minmax(kind == Fold::Max, ty, width);
                    if let Some(vec_op) = vec_op {
                        // src1/src2 lanes per the fold's operand order:
                        // arms_swapped selects (r, l) for the mirrored
                        // spellings — the FALSE arm must be src2.
                        let mut src1: Vec<Operand> = Vec::with_capacity(width);
                        let mut src2: Vec<Operand> = Vec::with_capacity(width);
                        let mut conds: Vec<Value> = Vec::with_capacity(width);
                        for v in &vals {
                            let (_, l, r, _, _, cv) = sel_shape(v).unwrap();
                            if arms_swapped {
                                src1.push(strip_identity_casts(ctx, &r));
                                src2.push(strip_identity_casts(ctx, &l));
                            } else {
                                src1.push(strip_identity_casts(ctx, &l));
                                src2.push(strip_identity_casts(ctx, &r));
                            }
                            conds.push(cv);
                        }
                        if let (Some(lhs), Some(rhs)) = (
                            build_pack(ctx, &src1, ty, width, fam, packs, dedup, depth + 1),
                            build_pack(ctx, &src2, ty, width, fam, packs, dedup, depth + 1),
                        ) {
                            let idx = packs.len();
                            packs.push(Pack {
                                kind: PackKind::FpMinMax {
                                    vec_op,
                                    lhs,
                                    rhs,
                                    cond_lanes: conds,
                                },
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
                // 3a. Rotate decomposition (RotateLeft/RotateRight lanes
                //     with a UNIFORM constant amount): the IR's own
                //     documented definition
                //         rotl(x, k)  = (x << k) | (x >> (W - k))
                //         rotr(x, k)  = rotl(x, W - k)
                //     holds per-lane BITWISE, so it holds on the packed
                //     value: the composite is one shl pack + one lshr pack
                //     + one or pack over the shared operand pack. x86 has
                //     no packed rotate before AVX-512, and this is exactly
                //     the triple GCC/Clang emit for vectorized ARX code.
                //     Amounts are normalized into [1, W-1] (the IR takes
                //     them modulo W; 0 is the identity and folds away);
                //     anything outside — including a non-constant or
                //     per-lane-varying amount — rejects these lanes.
                if matches!(op, IrBinOp::RotateLeft | IrBinOp::RotateRight) {
                    let bits = (ty.size() as i64) * 8;
                    let shl_op = packed_shift(IrBinOp::Shl, ty, width);
                    let shr_op = packed_shift(IrBinOp::LShr, ty, width);
                    let or_op = packed_binop(IrBinOp::Or, ty, width);
                    if let (Some(shl_op), Some(shr_op), Some(or_op)) = (shl_op, shr_op, or_op) {
                        let mut amount: Option<i64> = None;
                        let mut amounts_ok = true;
                        let mut lhs_lanes: Vec<Operand> = Vec::with_capacity(width);
                        for v in &vals {
                            let i = ctx.def_pos[&v.0];
                            let Instruction::BinOp { lhs, rhs, .. } = &block.instructions[i] else {
                                unreachable!()
                            };
                            let Some(k) = const_amount(ctx, rhs) else {
                                amounts_ok = false;
                                break;
                            };
                            // Normalize modulo the lane width (the IR's own
                            // rotate contract); a normalized 0 is the
                            // identity — not representable as the composite
                            // (W - 0 = W is outside the shift domain).
                            let kn = k.rem_euclid(bits);
                            if kn == 0 {
                                amounts_ok = false;
                                break;
                            }
                            match amount {
                                None => amount = Some(kn),
                                Some(a) if a == kn => {}
                                _ => {
                                    amounts_ok = false;
                                    break;
                                }
                            }
                            lhs_lanes.push(lhs.clone());
                        }
                        if amounts_ok {
                            let kn = amount.unwrap();
                            let rotl_k = if op == IrBinOp::RotateLeft {
                                kn
                            } else {
                                bits - kn
                            };
                            if let Some(val) =
                                build_pack(ctx, &lhs_lanes, ty, width, fam, packs, dedup, depth + 1)
                            {
                                let shl_idx = packs.len();
                                packs.push(Pack {
                                    kind: PackKind::ShiftImm {
                                        vec_op: shl_op,
                                        val,
                                        amount: rotl_k,
                                    },
                                    lane_vals: Vec::new(),
                                    sched: usize::MAX,
                                    order: 0,
                                });
                                let shr_idx = packs.len();
                                packs.push(Pack {
                                    kind: PackKind::ShiftImm {
                                        vec_op: shr_op,
                                        val,
                                        amount: bits - rotl_k,
                                    },
                                    lane_vals: Vec::new(),
                                    sched: usize::MAX,
                                    order: 0,
                                });
                                let idx = packs.len();
                                packs.push(Pack {
                                    kind: PackKind::BinOp {
                                        op: IrBinOp::Or,
                                        ty,
                                        vec_op: or_op,
                                        lhs: shl_idx,
                                        rhs: shr_idx,
                                    },
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
                // 3b. Uniform-constant shift lanes: one ShiftImm pack.
                //     The amount must be a compile-time constant, equal in
                //     every lane, and inside [1, W-1] — the defined C
                //     domain, where the packed immediate form is lane-exact
                //     against the scalar lowering by construction. Anything
                //     else (variable amount, per-lane amounts, 0, >= W)
                //     rejects these lanes.
                if matches!(op, IrBinOp::Shl | IrBinOp::LShr | IrBinOp::AShr) {
                    if let Some(vec_op) = packed_shift(op, ty, width) {
                        let bits = (ty.size() as i64) * 8;
                        let mut amount: Option<i64> = None;
                        let mut amounts_ok = true;
                        let mut lhs_lanes: Vec<Operand> = Vec::with_capacity(width);
                        for v in &vals {
                            let i = ctx.def_pos[&v.0];
                            let Instruction::BinOp { lhs, rhs, .. } = &block.instructions[i] else {
                                unreachable!()
                            };
                            let Some(k) = const_amount(ctx, rhs) else {
                                amounts_ok = false;
                                break;
                            };
                            if !(1..bits).contains(&k) {
                                amounts_ok = false;
                                break;
                            }
                            match amount {
                                None => amount = Some(k),
                                Some(a) if a == k => {}
                                _ => {
                                    amounts_ok = false;
                                    break;
                                }
                            }
                            lhs_lanes.push(lhs.clone());
                        }
                        if amounts_ok {
                            let k = amount.unwrap();
                            if let Some(val) =
                                build_pack(ctx, &lhs_lanes, ty, width, fam, packs, dedup, depth + 1)
                            {
                                let idx = packs.len();
                                packs.push(Pack {
                                    kind: PackKind::ShiftImm {
                                        vec_op,
                                        val,
                                        amount: k,
                                    },
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
                    // Rotate-pattern look-through (the raw C spelling):
                    // every lane is Or(Shl(x_i, k), LShr(x_i, W-k)) in
                    // either operand order, with the SAME x_i on both
                    // sides and a uniform k. The early SLP sweep runs
                    // BEFORE simplify canonicalizes the funnel-shift
                    // idiom into RotateLeft, so the raw spelling is the
                    // one that actually arrives here; recognizing it in
                    // the pack graph (instead of relying on phase order)
                    // builds ONE shared operand pack — the naive
                    // whole-side recursion builds two identical MemLoad
                    // packs, one per shift half, and the emitted code
                    // loads the vector twice.
                    //
                    // Soundness: identical to the RotateLeft composite —
                    // the pattern IS the IR's documented rotate
                    // definition, spelled out lane by lane, with the two
                    // sides reading the same SSA value.
                    if op == IrBinOp::Or {
                        let bits = (ty.size() as i64) * 8;
                        let shl_op = packed_shift(IrBinOp::Shl, ty, width);
                        let shr_op = packed_shift(IrBinOp::LShr, ty, width);
                        if let (Some(shl_op), Some(shr_op)) = (shl_op, shr_op) {
                            // (Shl(x,k), LShr(x,W-k)) in either order; the
                            // shared `same_source` (identity-cast strip +
                            // same-address-load proof) establishes the two
                            // halves read the same value.
                            let lane_rotate =
                                |lo: &Operand, hi: &Operand| -> Option<(i64, Operand)> {
                                    let lo = strip_identity_casts(ctx, lo);
                                    let hi = strip_identity_casts(ctx, hi);
                                    let side =
                                        |o: &Operand, want: IrBinOp| -> Option<(i64, Operand)> {
                                            let Operand::Value(v) = o else { return None };
                                            let i = ctx.def_pos.get(&v.0)?;
                                            let Instruction::BinOp {
                                                op: sop,
                                                lhs,
                                                rhs,
                                                ty: sty,
                                                ..
                                            } = &block.instructions[*i]
                                            else {
                                                return None;
                                            };
                                            if *sop != want || *sty != ty {
                                                return None;
                                            }
                                            let k = const_amount(ctx, rhs)?;
                                            Some((k, strip_identity_casts(ctx, lhs)))
                                        };
                                    let (lk, lx) = side(&lo, IrBinOp::Shl)?;
                                    let (rk, rx) = side(&hi, IrBinOp::LShr)?;
                                    // Same source, complementary amounts, both
                                    // in the defined shift domain.
                                    if !same_source(ctx, &lx, &rx)
                                        || lk + rk != bits
                                        || !(1..bits).contains(&lk)
                                    {
                                        return None;
                                    }
                                    Some((lk, lx))
                                };
                            let mut rot_amount: Option<i64> = None;
                            let mut rot_ok = true;
                            let mut rot_operands: Vec<Operand> = Vec::with_capacity(width);
                            for (l, r) in lhs_lanes.iter().zip(rhs_lanes.iter()) {
                                match lane_rotate(l, r).or_else(|| lane_rotate(r, l)) {
                                    Some((k, x)) => {
                                        match rot_amount {
                                            None => rot_amount = Some(k),
                                            Some(a) if a == k => {}
                                            _ => {
                                                rot_ok = false;
                                                break;
                                            }
                                        }
                                        rot_operands.push(x);
                                    }
                                    None => {
                                        rot_ok = false;
                                        break;
                                    }
                                }
                            }
                            if rot_ok && rot_amount.is_some() {
                                let k = rot_amount.unwrap();
                                if let Some(val) = build_pack(
                                    ctx,
                                    &rot_operands,
                                    ty,
                                    width,
                                    fam,
                                    packs,
                                    dedup,
                                    depth + 1,
                                ) {
                                    let shl_idx = packs.len();
                                    packs.push(Pack {
                                        kind: PackKind::ShiftImm {
                                            vec_op: shl_op,
                                            val,
                                            amount: k,
                                        },
                                        lane_vals: Vec::new(),
                                        sched: usize::MAX,
                                        order: 0,
                                    });
                                    let shr_idx = packs.len();
                                    packs.push(Pack {
                                        kind: PackKind::ShiftImm {
                                            vec_op: shr_op,
                                            val,
                                            amount: bits - k,
                                        },
                                        lane_vals: Vec::new(),
                                        sched: usize::MAX,
                                        order: 0,
                                    });
                                    let idx = packs.len();
                                    packs.push(Pack {
                                        kind: PackKind::BinOp {
                                            op: IrBinOp::Or,
                                            ty,
                                            vec_op,
                                            lhs: shl_idx,
                                            rhs: shr_idx,
                                        },
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
                    // Idiom rewrite: Sub(x, 1) → Add(x, -1). Bit-exact in
                    // two's complement for every lane value (x - 1 and
                    // x + (-1) wrap identically), and the -1 splat
                    // materializes as ONE pcmpeqd/vpcmpeqd where the 1
                    // splat pays the staged mov/movd/pshufd chain — the
                    // GCC/Clang `vpcmpeqd + vpaddd` idiom for `x - 1`.
                    // Only the rhs-constant-1 form: `1 - x` is a different
                    // (negation-shaped) beast and stays scalar.
                    let mut op = op;
                    let mut vec_op = vec_op;
                    if op == IrBinOp::Sub
                        && rhs_lanes
                            .iter()
                            .all(|o| matches!(o, Operand::Const(c) if c.to_i64() == Some(1)))
                    {
                        if let Some(add_op) = packed_binop(IrBinOp::Add, ty, width) {
                            if let Some(ones) = all_ones_const(ty) {
                                op = IrBinOp::Add;
                                vec_op = add_op;
                                for r in rhs_lanes.iter_mut() {
                                    *r = Operand::Const(ones.clone());
                                }
                            }
                        }
                    }
                    // Whole-side attempt, then (commutative) the swapped
                    // spelling. GATHER-AWARE: a side that degenerated to a
                    // gather is a low-quality success — the per-lane flip
                    // below may still find a packable arrangement, so the
                    // swap/flip attempts run whenever a side failed OR
                    // gathered, and a flip that eliminates gathers wins.
                    let is_gather = |packs: &[Pack], i: usize| {
                        matches!(
                            packs[i].kind,
                            PackKind::Gather2 { .. } | PackKind::Gather4 { .. }
                        )
                    };
                    let mut lhs_pack =
                        build_pack(ctx, &lhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                    let mut rhs_pack =
                        build_pack(ctx, &rhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                    let sides_gathered = |packs: &[Pack], l: Option<usize>, r: Option<usize>| {
                        [l, r].iter().flatten().any(|&i| is_gather(packs, i))
                    };
                    let needs_retry = lhs_pack.is_none()
                        || rhs_pack.is_none()
                        || sides_gathered(packs, lhs_pack, rhs_pack);
                    if is_commutative(op) && needs_retry {
                        let sw_lhs =
                            build_pack(ctx, &rhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                        let sw_rhs =
                            build_pack(ctx, &lhs_lanes, ty, width, fam, packs, dedup, depth + 1);
                        if sw_lhs.is_some() && sw_rhs.is_some() {
                            let improves = (lhs_pack.is_none() || rhs_pack.is_none())
                                || (sides_gathered(packs, lhs_pack, rhs_pack)
                                    && !sides_gathered(packs, sw_lhs, sw_rhs));
                            if improves {
                                lhs_pack = sw_lhs;
                                rhs_pack = sw_rhs;
                            }
                        }
                    }
                    // Per-lane commutative reordering (the LLVM
                    // Reassociate/SLP operand-shuffle subset): flip
                    // individual lanes to align operand KINDS —
                    // load-defined values on one side, constants on the
                    // other — so the sides form packable sets
                    // (consecutive-load runs, constant splats) that
                    // mixed spellings hid (`q[i] = a[i]*k + k*a[i]`
                    // chains: the identity/swap sides mix loads and
                    // scalars, degrade to gathers, and the cost model
                    // then rejects the whole seed). Sound because a
                    // commutative op computes the same lane value under
                    // any per-lane operand assignment. Bounded: one
                    // extra attempt pair, only when the flip actually
                    // changes the sides, preferred only when it removes
                    // gathers or fixes a failure.
                    let needs_retry = lhs_pack.is_none()
                        || rhs_pack.is_none()
                        || sides_gathered(packs, lhs_pack, rhs_pack);
                    if is_commutative(op) && needs_retry {
                        let kind = |o: &Operand| -> u8 {
                            match o {
                                Operand::Const(_) => 2,
                                Operand::Value(v) => {
                                    match ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]) {
                                        Some(Instruction::Load { .. }) => 0,
                                        _ => 1,
                                    }
                                }
                            }
                        };
                        let mut flipped = false;
                        let mut fa: Vec<Operand> = Vec::with_capacity(width);
                        let mut fb: Vec<Operand> = Vec::with_capacity(width);
                        for (l, r) in lhs_lanes.iter().zip(rhs_lanes.iter()) {
                            if kind(l) > kind(r) {
                                fa.push(r.clone());
                                fb.push(l.clone());
                                flipped = true;
                            } else {
                                fa.push(l.clone());
                                fb.push(r.clone());
                            }
                        }
                        if flipped {
                            let fl_lhs =
                                build_pack(ctx, &fa, ty, width, fam, packs, dedup, depth + 1);
                            let fl_rhs =
                                build_pack(ctx, &fb, ty, width, fam, packs, dedup, depth + 1);
                            if fl_lhs.is_some() && fl_rhs.is_some() {
                                let improves = (lhs_pack.is_none() || rhs_pack.is_none())
                                    || (sides_gathered(packs, lhs_pack, rhs_pack)
                                        && !sides_gathered(packs, fl_lhs, fl_rhs));
                                if improves {
                                    lhs_pack = fl_lhs;
                                    rhs_pack = fl_rhs;
                                }
                            }
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

    let mut root = build_pack(
        ctx,
        &cand.lane_ops,
        cand.ty,
        width,
        &fam,
        &mut packs,
        &mut dedup,
        0,
    )?;

    // ── Orphan pruning ─────────────────────────────────────────────
    // Failed recursion branches (whole-side, swap, flip attempts) leave
    // successfully-built sub-packs behind whose parent was never
    // created. Unpruned, such orphans would be scheduled, costed
    // (+width-1 each — INFLATING the benefit model), and emitted as
    // dead vector ops, and their lanes would be removed from the block.
    // Keep exactly what the root reaches.
    {
        let mut keep = vec![false; packs.len()];
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            if keep[i] {
                continue;
            }
            keep[i] = true;
            match packs[i].kind {
                PackKind::BinOp { lhs, rhs, .. } => {
                    stack.push(lhs);
                    stack.push(rhs);
                }
                PackKind::ShiftImm { val, .. } => {
                    stack.push(val);
                }
                PackKind::FpMinMax { lhs, rhs, .. } => {
                    stack.push(lhs);
                    stack.push(rhs);
                }
                _ => {}
            }
        }
        if keep.iter().any(|k| !k) {
            let mut remap = vec![usize::MAX; packs.len()];
            let mut new_packs: Vec<Pack> = Vec::with_capacity(packs.len());
            for (i, p) in packs.into_iter().enumerate() {
                if !keep[i] {
                    continue;
                }
                remap[i] = new_packs.len();
                new_packs.push(p);
            }
            for p in &mut new_packs {
                match &mut p.kind {
                    PackKind::BinOp { lhs, rhs, .. } => {
                        *lhs = remap[*lhs];
                        *rhs = remap[*rhs];
                    }
                    PackKind::ShiftImm { val, .. } => {
                        *val = remap[*val];
                    }
                    PackKind::FpMinMax { lhs, rhs, .. } => {
                        *lhs = remap[*lhs];
                        *rhs = remap[*rhs];
                    }
                    _ => {}
                }
            }
            packs = new_packs;
            root = remap[root];
        }
    }

    // ── Schedules ──────────────────────────────────────────────────────
    // Op/MemLoad packs: max lane def position (strictly before every
    // consumer — SSA within the block). Splat/gather leaves: min over
    // consumers, resolved by fixpoint — EXCEPT splats, which take their
    // EARLIEST valid position instead (see the splat block below).
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
    // Splats: EARLIEST-valid scheduling. A splat is pure, so the only
    // ordering constraints are (1) its scalar source's def and (2) the
    // block's phi prefix (a non-phi must never be inserted before a phi).
    // Placing splats at the earliest valid point — instead of the
    // latest (min over consumers) — puts every MemLoad pack ADJACENT to
    // the op pack that consumes it: the emitted IR reads
    //   [splat, ..., load, op]
    // instead of [load, splat, op], and the VLFOLD memory-fold analysis
    // (which requires the load next to — or next-but-one across another
    // pure load from — its consumer) fires for the streamed-load shapes
    // (`q[i] = a[i] - 1`, `~a[i]`, ARX quarter-rounds) exactly like
    // GCC/Clang schedule them: constant materialisation first, then
    // `vop (%rdi), %xmmK, %xmmD`. A value splat's source is defined
    // before the scalar lanes that consumed it (SSA), so
    // src_def+1 ≤ every consumer's schedule — never a violation.
    {
        // The prologue-like prefix: phis, ParamRefs and Allocas. A non-
        // prologue instruction inserted before a ParamRef breaks the
        // `x86_param_caller_homes_safe` prefix walk — the params then lose
        // their ABI-register homes and pay an entry copy + callee-save
        // push/pop pair each (measured on the ksub showdown shape: +6
        // instructions). Nothing but these three forms may precede the
        // first real instruction.
        let phi_prefix = block
            .instructions
            .iter()
            .position(|i| {
                !matches!(
                    i,
                    Instruction::Phi { .. }
                        | Instruction::ParamRef { .. }
                        | Instruction::Alloca { .. }
                )
            })
            .unwrap_or(block.instructions.len());
        for (pi, p) in packs.iter_mut().enumerate() {
            // The ROOT splat (an all-constant seed like `q[0..3] = 0`)
            // feeds only the seed STORE; early placement would needlessly
            // extend its live range across the whole block, so it keeps
            // the fixpoint/store-slot discipline.
            if pi == root {
                continue;
            }
            if let PackKind::Splat { src, .. } = &p.kind {
                let earliest = match src {
                    Operand::Value(v) => match ctx.def_pos.get(&v.0) {
                        Some(&d) => (d + 1).max(phi_prefix),
                        None => phi_prefix, // foreign value: dominates
                    },
                    Operand::Const(_) => phi_prefix,
                };
                p.sched = earliest;
            }
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
                    PackKind::BinOp { lhs, rhs, .. } => Some(vec![*lhs, *rhs]),
                    // A ShiftImm consumes its operand pack the same way a
                    // BinOp consumes its sides.
                    PackKind::ShiftImm { val, .. } => Some(vec![*val]),
                    PackKind::FpMinMax { lhs, rhs, .. } => Some(vec![*lhs, *rhs]),
                    _ => None,
                };
                if let Some(ins) = inputs {
                    if ins.contains(&i) {
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
    // Unresolved leaves mean a graph bug — with one exception: the ROOT
    // pack may itself be a Splat or Gather (empty `lane_vals`: an
    // all-same or gatherable seed like `q[0..3] = 0`, `q[0..3] = -1`).
    // Its ultimate consumer is the seed STORE, which the fixpoint above
    // cannot see (stores are not packs). The vector value must simply be
    // ready at the store's slot: default the root to the LAST seed store
    // position. Before this default, every constant-store seed bailed
    // here — `q[i] = 0` × N never vectorized at all (4 scalar stores or
    // backend pair-merges instead of one vpxor/vmovdqu).
    let store_max = *cand.store_idx.iter().max().unwrap();
    if packs[root].sched == usize::MAX {
        packs[root].sched = store_max;
    }
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
        // The FP min/max fold also removes the Cmp defs that fed the
        // folded Selects — their only remaining uses die with the select
        // lanes (an external use rejects the seed in the legality walk
        // below).
        if let PackKind::FpMinMax { cond_lanes, .. } = &p.kind {
            for c in cond_lanes {
                removed.insert(ctx.def_pos[&c.0]);
            }
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
        // The FP min/max fold's Cmp lanes: NO extract exists for them (a
        // scalar bool cannot be reconstructed from the packed mask), so
        // EVERY use outside the removed set — early OR late, terminator
        // included — rejects the seed. (The early-only variant left the
        // cmp's later consumers reading a deleted def: backend ICE.)
        if let PackKind::FpMinMax { cond_lanes, .. } = &p.kind {
            for c in cond_lanes {
                if ctx.external_uses.contains(&c.0) {
                    return None;
                }
                let uses = ctx.uses.get(&c.0).map(|us| us.as_slice()).unwrap_or(&[]);
                if !uses
                    .iter()
                    .all(|&u| u != usize::MAX && removed.contains(&u))
                {
                    return None;
                }
            }
        }
        // (c) No memory write strictly between the lane loads of a
        // MemLoad pack: an interleaved write could change which lanes see
        // old vs. new values. ESCAPE (rule (e)'s disjointness classes): a
        // write whose bytes are PROVABLY disjoint from the loaded window
        // [off0, off0 + width*size) cannot change any lane's observed
        // value — the restrict contract / global / alloca object
        // identity. Interleaved accumulations into other objects
        // (`q[i] = a[i]; acc += other[i];`) are the unblocked shape.
        // Same-stream writes get the exact byte-range test; anything not
        // a plain Store (calls, atomics, asm, vector intrinsics) keeps
        // the conservative rejection.
        if let PackKind::MemLoad { ptrs, .. } = &p.kind {
            let positions: Vec<usize> = p.lane_vals.iter().map(|v| ctx.def_pos[&v.0]).collect();
            let lo = *positions.iter().min().unwrap();
            let hi = *positions.iter().max().unwrap();
            let lane_addr = eval_sym_addr(block, &ctx.def_pos, ptrs[0]);
            let lane_hi = lane_addr
                .as_ref()
                .map(|la| la.off as i128 + width as i128 * fam.size as i128)
                .unwrap_or(0);
            for q in lo + 1..hi {
                if !is_memory_write(&block.instructions[q]) || removed.contains(&q) {
                    continue;
                }
                let (ptr, acc_size) = match &block.instructions[q] {
                    Instruction::Store { ptr, ty, .. } => (*ptr, ty.size() as i128),
                    _ => return None,
                };
                let disjoint_from_lanes =
                    match (&lane_addr, eval_sym_addr(block, &ctx.def_pos, ptr)) {
                        (Some(la), Some(wa))
                            if la.base == wa.base && la.var == wa.var && la.mult == wa.mult =>
                        {
                            wa.off as i128 >= lane_hi || la.off as i128 >= wa.off as i128 + acc_size
                        }
                        (Some(la), Some(wa)) => bases.disjoint(la.base.0, wa.base.0),
                        _ => false,
                    };
                if !disjoint_from_lanes {
                    return None;
                }
            }
        }
    }
    // (d) No memory access strictly between the seed stores other than
    // the removed lanes themselves: the vector store commits all lanes at
    // m_max, so an interleaved reader could observe a different
    // half-stored state. ESCAPE (rule (e)'s disjointness classes): a
    // scalar load/store whose bytes are PROVABLY disjoint from the seed
    // window [off0, off0 + width*size) cannot observe or modify the seed
    // bytes, so the batched commit is unobservable to it — the common
    // `pos[i] = x; total += other[i]; pos[i+1] = y;` shapes stay
    // vectorizable (GCC and Clang apply the same restrict/object-identity
    // reasoning). Same-stream accesses get the exact byte-range test;
    // foreign streams need the restrict/global/alloca disjointness proof;
    // everything unanalyzable (calls, atomics, asm, vector intrinsics,
    // opaque pointers) keeps the conservative rejection.
    let m_min = *cand.store_idx.iter().min().unwrap();
    let m_max = *cand.store_idx.iter().max().unwrap();
    let seed_addr = eval_sym_addr(block, &ctx.def_pos, cand.anchor_ptr);
    let seed_hi = seed_addr
        .as_ref()
        .map(|sa| sa.off as i128 + width as i128 * fam.size as i128)
        .unwrap_or(0);
    for q in m_min + 1..m_max {
        if removed.contains(&q) {
            continue;
        }
        let inst = &block.instructions[q];
        if !is_memory_access(inst) {
            continue;
        }
        let (ptr, acc_size) = match inst {
            Instruction::Load { ptr, ty, .. } | Instruction::Store { ptr, ty, .. } => {
                (*ptr, ty.size() as i128)
            }
            _ => return None,
        };
        let disjoint_from_seed = match (&seed_addr, eval_sym_addr(block, &ctx.def_pos, ptr)) {
            (Some(sa), Some(aa))
                if sa.base == aa.base && sa.var == aa.var && sa.mult == aa.mult =>
            {
                // Same stream: exact byte-range overlap.
                aa.off as i128 >= seed_hi || sa.off as i128 >= aa.off as i128 + acc_size
            }
            (Some(sa), Some(aa)) => bases.disjoint(sa.base.0, aa.base.0),
            _ => false,
        };
        if !disjoint_from_seed {
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
            PackKind::Splat { src, is_zero } => {
                // An integer all-ones splat is ONE instruction
                // (`try_all_ones_splat`'s self-compare) — the same cost
                // class as the zero splat, not the staged broadcast's 2.
                let is_ones = matches!(src,
                    Operand::Const(c) if c.to_i64() == Some(-1));
                benefit -= if *is_zero || is_ones { 1 } else { 2 };
            }
            PackKind::Gather2 { .. } => benefit -= 3,
            PackKind::Gather4 { .. } => benefit -= 7,
            // The FP min/max fold replaces W Selects AND their W Cmps with
            // one packed op; counted conservatively as one BinOp-level
            // replacement (the cmp removal is uncounted headroom).
            PackKind::FpMinMax { .. } => {
                benefit += width as i64 - 1;
            }
            // Plain scalar shift lanes replace W ops with one vector op
            // (+W-1); the synthetic halves of a rotate decomposition
            // replace nothing (cost 1 each, like a splat leaf).
            PackKind::ShiftImm { .. } => {
                if p.lane_vals.is_empty() {
                    benefit -= 1;
                } else {
                    benefit += width as i64 - 1;
                }
            }
        }
    }
    // Extract costs: 1 move for a low-half lane; a 256-bit HIGH-half
    // lane pays the vextracti128/f128 staging first (2 total).
    let is_256 = fam.size * width as u64 == 32;
    let half_lanes = width / 2;
    for (_, (_, li)) in &extract_of {
        benefit -= 1 + (is_256 && *li >= half_lanes) as i64;
    }
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
        PackKind::ShiftImm { val, .. } => 1 + topo_depth(packs, *val),
        PackKind::FpMinMax { lhs, rhs, .. } => {
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
            PackKind::ShiftImm {
                vec_op,
                val,
                amount,
            } => Instruction::Intrinsic {
                dest: Some(dest),
                op: *vec_op,
                dest_ptr: None,
                args: vec![
                    Operand::Value(vec_dest[*val]),
                    Operand::Const(IrConst::I64(*amount)),
                ],
            },
            PackKind::FpMinMax {
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
        let kinds: Vec<String> = plan
            .packs
            .iter()
            .map(|p| match &p.kind {
                PackKind::MemLoad { .. } => "MemLoad".into(),
                PackKind::BinOp { op, lhs, rhs, .. } => {
                    format!("BinOp({:?} l{} r{})", op, lhs, rhs)
                }
                PackKind::Splat { .. } => "Splat".into(),
                PackKind::Gather2 { .. } => "Gather2".into(),
                PackKind::Gather4 { .. } => "Gather4".into(),
                PackKind::ShiftImm { val, amount, .. } => {
                    format!("ShiftImm(v{} ${})", val, amount)
                }
                PackKind::FpMinMax { .. } => "FpMinMax".into(),
            })
            .collect();
        eprintln!(
            "[BB-SLP] fn={} block B{}: packed {}x{:?} ({} packs, benefit {}): [{}]",
            func.name,
            block_idx,
            width,
            cand.ty,
            plan.packs.len(),
            plan.benefit,
            kinds.join(", ")
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
