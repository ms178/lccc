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
            // Base of a GEP may itself be a GEP — fold recursively. A
            // base that carries its own index variable now COMPOSES:
            //
            //   * const offset over a var-carrying base — the struct-
            //     field pattern `a[i].f2` is GEP(GEP(a, i*stride),
            //     field_off). Degradation (the previous behavior) put
            //     `a[i].f1` in stream (a, i, stride, 0) but `a[i].f2`
            //     in stream (GEP(a,i*stride), -, 1, field_off) — two
            //     DIFFERENT stream keys for accesses of the SAME
            //     object and index, so the field pair never grouped
            //     and struct-array kernels (nbody bodies[i].vx/vy,
            //     rbtree node fields, Particle x/y/z) never seeded.
            //     Composition keeps the ROOT symbol as the base —
            //     strictly more canonical, and strictly better for
            //     `RestrictBases::disjoint` (a root GlobalAddr/alloca/
            //     param classifies; an intermediate GEP value never
            //     did).
            //   * var offset indexing the SAME variable — the byte
            //     offsets compose scale-wise (base + m1·v + m2·v);
            //     both scales are positive by `affine_term`'s
            //     canonicality, and the sum is u64-checked (fail-closed
            //     to None on overflow).
            //   * var offset with a DIFFERENT variable — two index
            //     variables in one address (`a[i][j]`): no single-
            //     stream model. The base degrades to the inner GEP's
            //     value as an opaque anchor and THIS GEP's index
            //     becomes the stream variable (two accesses through
            //     the SAME base-GEP value still stream together — the
            //     `p = &a[i]; p[0..3]` shape; the historical behavior,
            //     byte-for-byte).
            let mut addr = eval_sym_addr_d(block, def_pos, *base, depth.saturating_sub(1))
                .unwrap_or(SymAddr {
                    base: *base,
                    var: None,
                    mult: 1,
                    off: 0,
                });
            // The offset term: a constant folds into `off`; a variable
            // term (with its scale and constant part from `affine_term`)
            // becomes the stream's index. `addr` may carry at most one
            // index variable at this point; the same variable composes,
            // a different one degrades the base (below).
            let t = affine_term(block, def_pos, offset, AFFINE_DEPTH)?;
            match t.var {
                None => {
                    addr.off = addr.off.checked_add(t.off)?;
                }
                Some(v) => match addr.var {
                    None => {
                        addr.var = Some(v);
                        addr.mult = t.mult as u64;
                        addr.off = addr.off.checked_add(t.off)?;
                    }
                    Some(bv) if bv == v => {
                        // Same index variable at both levels: the address
                        // is base + (m1 + m2)·v + (o1 + o2). Both mults
                        // are positive (canonical affine terms); a sum
                        // that overflows u64 was never a real stride.
                        let m = i64::try_from(addr.mult).ok()?.checked_add(t.mult)?;
                        if m <= 0 {
                            return None;
                        }
                        addr.mult = m as u64;
                        addr.off = addr.off.checked_add(t.off)?;
                    }
                    Some(_) => {
                        // Two different index variables (`a[i][j]`): the
                        // base becomes the opaque anchor and this GEP's
                        // index is the stream variable.
                        addr = SymAddr {
                            base: *base,
                            var: Some(v),
                            mult: t.mult as u64,
                            off: t.off,
                        };
                    }
                },
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
        // HALF-WIDE dword pairs: 2 lanes of 4 bytes in the low 64 bits of
        // an XMM register. The register SHAPE is I32x4 — every
        // lane-independent op between the endpoints resolves (via
        // `reg_width_for`) to the ordinary I32x4 intrinsics, so the value
        // web, homing classes and pack graph stay the familiar full-width
        // ones; only the MEMORY endpoints (load/store/gather) are the
        // 64-bit Pair forms with the upper lanes deterministically zero.
        // This is GCC's SHA-256 message-schedule shape: `vmovq` pair
        // loads feeding `vpsrld/vpxor/vpaddd`.
        (IrType::I32 | IrType::U32, 2) => Some(VecFamily {
            load: IntrinsicOp::VecLoadI32x4Pair,
            store: IntrinsicOp::VecStoreI32x4Pair,
            broadcast: IntrinsicOp::VecBroadcastI32x4,
            zero: IntrinsicOp::VecZeroI32x4,
            pack2: Some(IntrinsicOp::VecPackI32x4Pair),
            pack4: None,
            extract: Some(IntrinsicOp::VecExtractLaneI32x4),
            size: 4,
        }),
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
    let width = reg_width_for(ty, width);
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

/// Map an FP lane type/width to its packed bitwise-XOR intrinsic (the
/// exact packed spelling of every FP bit-manipulation idiom whose scalar
/// lowering is an integer-domain XOR on the bit pattern).
fn packed_fpxor(ty: IrType, width: usize) -> Option<IntrinsicOp> {
    match (ty, width) {
        (IrType::F32, 8) => Some(IntrinsicOp::VecXorF32x8),
        (IrType::F32, 4) => Some(IntrinsicOp::VecXorF32x4),
        (IrType::F64, 4) => Some(IntrinsicOp::VecXorF64x4),
        (IrType::F64, 2) => Some(IntrinsicOp::VecXorF64x2),
        _ => None,
    }
}

/// The `-0.0` constant of an FP lane type — the sign-mask operand of the
/// FP-Neg composite (bit pattern: sign bit set, everything else clear).
fn minus_zero_const(ty: IrType) -> Option<IrConst> {
    match ty {
        IrType::F32 => Some(IrConst::F32(-0.0)),
        IrType::F64 => Some(IrConst::F64(-0.0)),
        _ => None,
    }
}

/// Integer lane min/max packed intrinsics. Exact and commutative (no FP
/// unordered/±0 asymmetry) — every spelling of the same-source ternary
/// folds. Dword requires SSE4.1 (pminsd/pmaxsd); gate on AVX2 — the SLP
/// pass's only feature accessor — which implies it (x86-64-v2-only builds
/// simply keep the cmp+blendv two-op form, which is itself exact).
/// Halfword (pminsw/pmaxsw) and byte (pminub/pmaxub) are SSE2.
fn packed_int_minmax(is_max: bool, ty: IrType, width: usize) -> Option<IntrinsicOp> {
    let width = reg_width_for(ty, width);
    let avx2 = x86_avx2_available_pub();
    match (ty, width) {
        (IrType::I32 | IrType::U32, 8) => Some(if is_max {
            IntrinsicOp::VecMaxI32x8
        } else {
            IntrinsicOp::VecMinI32x8
        }),
        (IrType::I32 | IrType::U32, 4) if avx2 => Some(if is_max {
            IntrinsicOp::VecSmaxI32x4
        } else {
            IntrinsicOp::VecSminI32x4
        }),
        (IrType::I16 | IrType::U16, 16) => Some(if is_max {
            IntrinsicOp::VecMaxI16x16
        } else {
            IntrinsicOp::VecMinI16x16
        }),
        (IrType::I16 | IrType::U16, 8) => Some(if is_max {
            IntrinsicOp::VecMaxI16x8
        } else {
            IntrinsicOp::VecMinI16x8
        }),
        // Unsigned bytes: pminub/pmaxub (SSE2). Signed bytes have no
        // pre-AVX512 packed min/max.
        (IrType::U8, 16) => Some(if is_max {
            IntrinsicOp::VecMaxU8x16
        } else {
            IntrinsicOp::VecMinU8x16
        }),
        (IrType::U8, 32) => Some(if is_max {
            IntrinsicOp::VecMaxU8x32
        } else {
            IntrinsicOp::VecMinU8x32
        }),
        _ => None,
    }
}

/// The packed compare + lane-mask-select intrinsic pair for a (lane type,
/// width) family. `None` where no family exists (I64 lanes) — the lanes
/// then degrade to the gather/binop paths.
fn packed_cmp_blendv(ty: IrType, width: usize) -> Option<(IntrinsicOp, IntrinsicOp)> {
    let width = reg_width_for(ty, width);
    let avx2 = x86_avx2_available_pub();
    match (ty, width) {
        (IrType::F32, 8) if avx2 => Some((IntrinsicOp::VecCmpF32x8, IntrinsicOp::VecBlendvF32x8)),
        (IrType::F32, 4) => Some((IntrinsicOp::VecCmpF32x4, IntrinsicOp::VecBlendvF32x4)),
        (IrType::F64, 4) if avx2 => Some((IntrinsicOp::VecCmpF64x4, IntrinsicOp::VecBlendvF64x4)),
        (IrType::F64, 2) => Some((IntrinsicOp::VecCmpF64x2, IntrinsicOp::VecBlendvF64x2)),
        (IrType::I32 | IrType::U32, 8) if avx2 => {
            Some((IntrinsicOp::VecCmpI32x8, IntrinsicOp::VecBlendvI32x8))
        }
        (IrType::I32 | IrType::U32, 4) => {
            Some((IntrinsicOp::VecCmpI32x4, IntrinsicOp::VecBlendvI32x4))
        }
        (IrType::I16 | IrType::U16, 16) if avx2 => {
            Some((IntrinsicOp::VecCmpI16x16, IntrinsicOp::VecBlendvI16x16))
        }
        (IrType::I16 | IrType::U16, 8) => {
            Some((IntrinsicOp::VecCmpI16x8, IntrinsicOp::VecBlendvI16x8))
        }
        (IrType::I8 | IrType::U8, 32) if avx2 => {
            Some((IntrinsicOp::VecCmpI8x32, IntrinsicOp::VecBlendvI8x32))
        }
        (IrType::I8 | IrType::U8, 16) => {
            Some((IntrinsicOp::VecCmpI8x16, IntrinsicOp::VecBlendvI8x16))
        }
        _ => None,
    }
}

/// The packed-compare predicate immediate for a scalar comparison, with
/// the operand-swap flag for the mirrored relations. FP vocabulary:
/// 0 = EQ_OQ, 1 = LT_OS, 2 = LE_OS, 4 = NEQ_UQ (exactly the C spellings —
/// all false on NaN). Integer vocabulary: 0 = eq, 1 = lt.s, 2 = le.s,
/// 4 = ne, 5 = lt.u, 6 = le.u. Unsigned relations are integer-only.
fn cmp_predicate_imm(op: IrCmpOp, is_fp: bool) -> Option<(i64, bool)> {
    match op {
        IrCmpOp::Eq => Some((0, false)),
        IrCmpOp::Ne => Some((4, false)),
        IrCmpOp::Slt => Some((1, false)),
        IrCmpOp::Sle => Some((2, false)),
        IrCmpOp::Sgt => Some((1, true)),
        IrCmpOp::Sge => Some((2, true)),
        IrCmpOp::Ult if !is_fp => Some((5, false)),
        IrCmpOp::Ule if !is_fp => Some((6, false)),
        IrCmpOp::Ugt if !is_fp => Some((5, true)),
        IrCmpOp::Uge if !is_fp => Some((6, true)),
        _ => None,
    }
}

/// Map a shift lane op to its packed intrinsic — the uniform CONSTANT
/// amount forms only (`psllw/psrlw/psraw/pslld/psrld/psrad/psllq/psrlq`
/// and their VEX counterparts). None where the ISA has no packed form:
/// byte lanes (no packed byte shift before AVX-512) and I64 arithmetic
/// right shift (no packed qword `psraq` before AVX-512). Those lanes
/// degrade to gathers where a family has one, else reject the seed.
fn packed_shift(op: IrBinOp, ty: IrType, width: usize) -> Option<IntrinsicOp> {
    let width = reg_width_for(ty, width);
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
    let width = reg_width_for(ty, width);
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

/// Strip casts that preserve the BIT PATTERN exactly: same-width integer
/// reinterprets (`(int32_t)uint32_t` — U32→I32 and every sibling
/// spelling) plus the `from == to` identity casts `strip_identity_casts`
/// covers. Legal wherever the consumer reasons about RAW BITS and takes
/// its signedness from the predicate, not the cast types: the packed
/// compares read the identical lanes, and the min/max arm identity is a
/// bit-level question. A widening or narrowing cast is NEVER stripped
/// here (the bits differ).
fn strip_bitidentity_casts(ctx: &BlockCtx, o: &Operand) -> Operand {
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
            } if from_ty == to_ty
                || (ty_is_integer(*from_ty)
                    && ty_is_integer(*to_ty)
                    && from_ty.size() == to_ty.size()) =>
            {
                cur = src.clone();
            }
            _ => break,
        }
    }
    cur
}

/// Remap a promoted-width compare predicate onto the narrow lane type it
/// was widened from, given the widening cast's source and destination
/// signedness. Exactness:
///   * Eq/Ne are width-free.
///   * A zext widening (unsigned source) makes the promoted values
///     non-negative: a SIGNED promoted compare then agrees with the
///     UNSIGNED narrow compare (this is exactly C's promotion of
///     unsigned char/short lanes to `int`), and an unsigned one with
///     itself.
///   * A sext widening (signed source into a SIGNED wider type) is
///     strictly monotone for the signed order: signed predicates keep.
///     A sext into an UNSIGNED wider type reorders (the sign extension
///     enters the unsigned domain as a huge value): reject. An unsigned
///     predicate over sext values reorders the same way: reject.
fn demote_cmp_pred(op: IrCmpOp, from_unsigned: bool, to_unsigned: bool) -> Option<IrCmpOp> {
    match op {
        IrCmpOp::Eq | IrCmpOp::Ne => Some(op),
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => {
            if from_unsigned {
                Some(match op {
                    IrCmpOp::Slt => IrCmpOp::Ult,
                    IrCmpOp::Sle => IrCmpOp::Ule,
                    IrCmpOp::Sgt => IrCmpOp::Ugt,
                    _ => IrCmpOp::Uge,
                })
            } else if !to_unsigned {
                Some(op)
            } else {
                None
            }
        }
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => {
            if from_unsigned {
                Some(op)
            } else {
                None
            }
        }
    }
}

/// The narrow-lane constant for a promoted integer constant: the demoted
/// select's arm/compare operand. ARM constants truncate exactly (the
/// stored value is `trunc(promoted select)` for every arm value); COMPARE
/// operands additionally require the value to lie in the narrow range of
/// the final predicate's signedness (checked by the caller — an
/// out-of-range compare operand changes the predicate's truth value).
fn narrow_const(cv: i64, ty: IrType) -> IrConst {
    match ty {
        IrType::I8 | IrType::U8 => IrConst::I8(cv as i8),
        _ => IrConst::I16(cv as i16),
    }
}

/// Does `cv` lie in the narrow lane's range for a predicate of
/// signedness `unsigned`? Outside it, the promoted compare is
/// constant-foldable and the demotion must not guess its answer.
fn fits_narrow(cv: i64, ty: IrType, unsigned: bool) -> bool {
    let bits = 8 * ty.size() as i64;
    if unsigned {
        (0..1i64 << bits).contains(&cv)
    } else {
        (-(1i64 << (bits - 1))..1i64 << (bits - 1)).contains(&cv)
    }
}

/// Two operands name the same SOURCE when (after bit-identity cast
/// stripping — same-width integer reinterprets preserve every bit, so
/// they name the same loadable value) they are the same value, or two
/// loads of the same symbolic address with NO memory write between them
/// — the pre-CSE frontend emits one load per spelling side, and the
/// interval check is the whole same-value proof (any write between the
/// two loads could change the observed bytes; rule (c) covers the
/// pack's own lane range only).
fn same_source(ctx: &BlockCtx, a: &Operand, b: &Operand) -> bool {
    let a = strip_bitidentity_casts(ctx, a);
    let b = strip_bitidentity_casts(ctx, b);
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

/// The SAME-SOURCE proof for the same-source SPLAT (pack 1a): two
/// operands (already bit-identity-stripped by the caller) name the same
/// loadable value when they are the same SSA value, or non-volatile
/// Default-segment loads of the SAME symbolic address — same base, same
/// index variable, same stride, same offset — with no write between them
/// that touches those bytes.
///
/// This is `same_source`'s proof with the conservative "any write
/// between the loads rejects" interval upgraded to the full symbolic
/// disjointness battery: a write to the SAME stream is checked by exact
/// byte ranges, a write to the same base with the same stride but a
/// different index variable by the FIELD-DISJOINTNESS THEOREM, and
/// everything else still rejects. That upgrade is what the pre-CSE
/// struct-field spelling needs: `bodies[j].mass` is loaded once per
/// component with velocity stores to `bodies[i].vx/vy/vz` in between —
/// stores that provably never touch the mass field for ANY i, j.
fn same_source_loads_symbolic(ctx: &BlockCtx, a: &Operand, b: &Operand) -> bool {
    if a == b {
        return true;
    }
    let block = ctx.block;
    let addr_of = |o: &Operand| -> Option<(SymAddr, usize, i64)> {
        let Operand::Value(v) = o else { return None };
        let &i = ctx.def_pos.get(&v.0)?;
        match &block.instructions[i] {
            Instruction::Load {
                ptr,
                seg_override,
                volatile,
                ty,
                ..
            } if *seg_override == AddressSpace::Default && !*volatile => {
                eval_sym_addr(block, &ctx.def_pos, *ptr).map(|ad| (ad, i, ty.size() as i64))
            }
            _ => None,
        }
    };
    let Some((aa, pa, sa)) = addr_of(a) else {
        return false;
    };
    let Some((ab, pb, sb)) = addr_of(b) else {
        return false;
    };
    if aa.base != ab.base || aa.var != ab.var || aa.mult != ab.mult || aa.off != ab.off {
        return false;
    }
    let size = sa.max(sb);
    let (plo, phi) = if pa < pb { (pa, pb) } else { (pb, pa) };
    (plo + 1..phi).all(|q| {
        let inst = &block.instructions[q];
        if !is_memory_write(inst) {
            return true;
        }
        let Instruction::Store {
            ptr,
            ty: wty,
            seg_override,
            volatile,
            ..
        } = inst
        else {
            return false;
        };
        if *seg_override != AddressSpace::Default || *volatile {
            return false;
        }
        let Some(wa) = eval_sym_addr(block, &ctx.def_pos, *ptr) else {
            return false;
        };
        let ws = wty.size() as i64;
        if wa.base == aa.base && wa.var == aa.var && wa.mult == aa.mult {
            // Same stream: exact byte ranges.
            !byte_ranges_overlap(aa.off, size, wa.off, ws)
        } else if wa.base == aa.base && wa.mult == aa.mult {
            // Same base and stride, different (or absent) index variable:
            // the field-disjointness theorem. `aa.mult` is the shared
            // stride; a mult of 1 with distinct constant windows is the
            // degenerate "no stride" case the byte-range arm already
            // covers when the vars match, and the theorem handles the
            // rest.
            field_disjoint(aa.mult, aa.off, size, wa.off, ws)
        } else {
            // Different base or stride: no symbolic proof available
            // here (this helper has no RestrictBases access — the pack
            // rules (c)/(d)/(e) apply those classes separately).
            false
        }
    })
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

/// THE FIELD-DISJOINTNESS THEOREM (same base, same stride, different
/// index variables).
///
/// Two accesses `base + m·i + off1` (width `s1`) and `base + m·j + off2`
/// (width `s2`) — with the SAME base object and the SAME positive byte
/// stride `m`, but INDEPENDENT index variables `i`, `j` — never overlap
/// when both accesses are confined to single elements and their
/// element-relative windows are disjoint:
///
/// * normalize `o1 = off1 mod m`, `o2 = off2 mod m` (both in `[0, m)`),
/// * require `o1 + s1 ≤ m` and `o2 + s2 ≤ m` (no access spans an
///   element boundary), and `o1 + s1 ≤ o2 || o2 + s2 ≤ o1`.
///
/// PROOF: the address difference is `(o1 − o2) + m·d` with
/// `d = i − j ∈ ℤ` and `v0 = o1 − o2 ∈ (−m, m)`. Overlap of
/// `[A, A+s1)` and `[B, B+s2)` needs `−s1 < A − B < s2`, i.e.
/// `−s1 < v0 + m·d < s2`.
/// * `d = 0`: in-element disjointness gives `v0 ≤ −s1` or `v0 ≥ s2` —
///   excluded.
/// * `d ≥ 1`: `v0 + m·d ≥ v0 + m > −m + m = 0`, and `v0 + m·d < s2 ≤ m`
///   needs `v0 < s2 − m·d ≤ s2 − m`. But `v0 ≥ −o2 ≥ −(m − s2) = s2 − m`
///   (single-element confinement of access 2) — excluded. For `d ≥ 2`,
///   `v0 + m·d ≥ v0 + 2m > m ≥ s2` outright.
/// * `d ≤ −1`: symmetric — `v0 + m·d ≤ v0 − m < m − m = 0`, and
///   `> −s1` needs `v0 > m − s1`; but `v0 ≤ o1 ≤ m − s1` (single-element
///   confinement of access 1) — excluded. For `d ≤ −2`,
///   `v0 + m·d ≤ v0 − 2m < −m ≤ −s1` outright.
///
/// This is the proof that makes struct-field streams independent of the
/// index relation: `bodies[i].mass` (element window [48,56) of a 56-byte
/// element) and `bodies[j].vx` (window [24,32)) can never alias for ANY
/// `i`, `j` — including `i == j` — which no amount of same-stream
/// offset comparison could establish (the streams carry different index
/// variables). Without it, every intervening field access of a struct
/// array rejected rules (c)/(d) and the same-source splat below.
fn field_disjoint(mult: u64, off1: i64, size1: i64, off2: i64, size2: i64) -> bool {
    let m = mult as i128;
    if m <= 0 {
        return false;
    }
    let s1 = size1 as i128;
    let s2 = size2 as i128;
    if s1 <= 0 || s2 <= 0 || s1 > m || s2 > m {
        return false;
    }
    let o1 = ((off1 as i128) % m + m) % m;
    let o2 = ((off2 as i128) % m + m) % m;
    if o1 + s1 > m || o2 + s2 > m {
        return false;
    }
    o1 + s1 <= o2 || o2 + s2 <= o1
}

/// Byte-range overlap of two same-stream windows at constant offsets.
fn byte_ranges_overlap(off1: i64, size1: i64, off2: i64, size2: i64) -> bool {
    let a = off1 as i128;
    let b = off2 as i128;
    a < b + size2 as i128 && b < a + size1 as i128
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
    /// The lanes are lane-extracts of ONE existing vector value at
    /// consecutive indices 0..width-1 — the pack IS that vector. This is
    /// the CHAINED-SEED forward: a second seed whose operand tree reads
    /// a first seed's vector through its extracts (the nbody pair body:
    /// the i-velocity seed packs (dx,dy) into one vsubpd; the
    /// j-velocity seed's mul tree consumes the SAME (dx,dy) through the
    /// extracts the rewrite left behind) reuses the vector with ZERO
    /// new instructions instead of re-gathering the scalars. Without
    /// the forward, the second seed's operand recursion meets extract
    /// values (not loads, not binops) and falls to the low-benefit
    /// gather — or declines outright.
    Forward { val: Value },
    /// Same-op binop lanes. `ty` is the lane type (== the binop type).
    BinOp {
        op: IrBinOp,
        ty: IrType,
        vec_op: IntrinsicOp,
        lhs: usize,
        rhs: usize,
    },
    /// Packed FMA/FMS contraction: every lane is `acc_lane ± Mul(x_lane,
    /// s_lane)` with the `s` side UNIFORM across lanes (bit-identical or
    /// same-source — a Splat pack) — emitted as ONE vfmadd/vfnmadd
    /// reading the accumulator pack, the varying multiplicand pack and
    /// the splat. ROUNDING PARITY: the scalar path contracts the same
    /// shape (the gap-fused mul+add/sub detector), so the vector form
    /// MUST fuse the outer multiply too — a packed mulpd+subpd pair pays
    /// a THIRD rounding the scalar code never takes and the tri-config
    /// differential (SLP on / SLP off / gcc) drifts. FMA3-gated at pack
    /// build; the FP contract is threaded from the driver (Fast for C,
    /// exactly the scalar contraction's policy).
    Fma {
        /// false: dest = acc + a·b (vfmadd); true: dest = acc − a·b
        /// (vfnmadd).
        negate: bool,
        acc: usize,
        a: usize,
        b: usize,
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
    /// FP negation composite: `dest = VecXorF*(val, Const(-0.0))` — the
    /// emitter folds the per-lane sign mask as a .rodata memory operand,
    /// ONE instruction for the whole pack (the exact GCC spelling of
    /// packed `-x` lanes). Bit-exactness proof: the scalar lowering
    /// (alu.rs `emit_float_neg_impl`) XORs the GPR bit pattern with the
    /// sign mask — a pure sign-bit flip, NOT `0.0 - x` — identical to the
    /// packed XOR for every lane value incl. NaN payloads and ±0.
    FpNeg { vec_op: IntrinsicOp, val: usize },
    /// General compare+select composite: TWO intrinsics — a packed
    /// compare (all-ones/all-zeros lane mask; `pred` follows the VecCmp*
    /// immediate vocabulary) feeding a bitwise lane select
    /// (`blendv args = [fv, tv, mask]`). Replaces W scalar cmp+select
    /// pairs for EVERY predicate and mixed arms (the shapes the min/max
    /// fold cannot take). `cond_lanes` are the Cmp defs that die with the
    /// folded Select lanes (removed; every non-removed use rejects the
    /// seed — a scalar bool cannot be reconstructed from the packed
    /// mask cheaply).
    CmpBlendv {
        cmp_op: IntrinsicOp,
        blend_op: IntrinsicOp,
        pred: i64,
        lhs: usize,
        rhs: usize,
        tv: usize,
        fv: usize,
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
    run_bb_slp_with_contract(
        func,
        crate::common::fp_contract::FpContract::c_language_default(),
    )
}

/// The FP-contraction-aware entry: the packed FMA contraction must honor
/// exactly the contract the scalar gap-fused detector sees (threaded from
/// the driver's -ffp-contract flag). The bare `run_bb_slp` above keeps the
/// `run_on_visited` closure shape with the C default.
pub(crate) fn run_bb_slp_with_contract(
    func: &mut IrFunction,
    fp_contract: crate::common::fp_contract::FpContract,
) -> usize {
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
            while slp_block_once(func, b, &cross, fp_contract, debug) == 1 {
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
    /// The FP contraction contract (threaded from the driver): gates the
    /// packed FMA contraction exactly like the scalar gap-fused detector.
    fp_contract: crate::common::fp_contract::FpContract,
}

fn build_ctx<'a>(
    func: &'a IrFunction,
    block_idx: usize,
    cross: &'a FxHashSet<u32>,
    fp_contract: crate::common::fp_contract::FpContract,
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
        fp_contract,
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
                        // Fallback sub-window candidates: the full-width
                        // seed takes the first `width` stores of the run,
                        // and when ITS lanes are not uniform (v7_pressure:
                        // three independent 4-lane groups — add, sub, xor
                        // — fused into one 12-store run) the pack build
                        // fails and, without fallbacks, the WHOLE run
                        // stays scalar. Every aligned 128-bit window is
                        // offered as its own seed so the uniform groups
                        // still pack (GCC's behavior on the same shape).
                        // Overlap with the primary seed is safe: plans are
                        // attempted longest-first, one applied per scan,
                        // and the rewrite turns the packed stores into
                        // vector stores before the rescan re-collects.
                        let w128 = (16 / size) as usize;
                        if width > w128 {
                            for chunk in run.chunks(w128) {
                                if chunk.len() < 2 {
                                    continue;
                                }
                                let cw = match preferred_width(chunk.len(), size as u64, ty, avx2) {
                                    Some(cw) if cw == chunk.len() => cw,
                                    _ => continue,
                                };
                                let store_idx: Vec<usize> =
                                    chunk.iter().map(|(_, i, _, _)| *i).collect();
                                let lane_ops: Vec<Operand> =
                                    chunk.iter().map(|(_, _, v, _)| v.clone()).collect();
                                candidates.push(SeedCandidate {
                                    store_idx,
                                    lane_ops,
                                    ty,
                                    anchor_ptr: chunk[0].3,
                                    width: cw,
                                });
                            }
                        }
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
    } else if size == 4 && len == 2 && family_for(ty, 2).is_some() {
        // HALF-WIDE dword pairs: two consecutive dword stores are one
        // 64-bit store of the low half of an XMM register (GCC's 2-wide
        // message-schedule / stream-transform shape). Reaches here only
        // when the full-width families did not take the run (len < 4).
        Some(2)
    } else {
        None
    }
}

/// Normalize an SLP pack width to the REGISTER width its ops use.
///
/// HALF-WIDE packs (2 lanes of 4-byte integers) live in the low 64 bits
/// of an XMM register whose full shape is 4 lanes: every lane-independent
/// op between the 64-bit memory endpoints is the ordinary full-register
/// intrinsic, so the op resolvers must look up (ty, REGISTER width), not
/// (ty, pack width). The upper lanes hold `op(0, 0)` (the Pair load and
/// gather zero them; VEX/SSE `movq`/`movd` both clear the high bits) —
/// deterministic, and never observable because the Pair store writes
/// only lanes 0/1.
fn reg_width_for(ty: IrType, width: usize) -> usize {
    match (ty, width) {
        (IrType::I32 | IrType::U32, 2) => 4,
        _ => width,
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
    fp_contract: crate::common::fp_contract::FpContract,
    debug: bool,
) -> usize {
    // All analysis under one immutable borrow; the plans are fully owned
    // (no lifetimes into the block), so the mutable rewrite afterwards
    // is borrow-clean.
    let (candidates, plans): (Vec<SeedCandidate>, Vec<Option<Plan>>) = {
        let ctx = build_ctx(func, block_idx, cross, fp_contract);
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
/// Cross-seed splat CSE lookup: does this block already contain a
/// `fam.broadcast` (or the zero form for all-zero constants) of exactly
/// this source operand (bit-identity)? Returns the broadcast's dest —
/// the vector a new Splat pack would reproduce. Only same-block hits
/// are useful (the Forward pack schedules at the dest's position and
/// consumers are later in this block by construction).
fn find_existing_broadcast(ctx: &BlockCtx, fam: &VecFamily, src: &Operand) -> Option<Value> {
    let block = ctx.block;
    let zero_src = matches!(src, Operand::Const(c) if c.is_all_zero_bits());
    for inst in &block.instructions {
        let Instruction::Intrinsic {
            dest: Some(d),
            op,
            args,
            dest_ptr: None,
        } = inst
        else {
            continue;
        };
        let is_bcast = if zero_src {
            // The zero splat's emitted form is fam.zero with no args.
            *op == fam.zero && args.is_empty()
        } else {
            *op == fam.broadcast && args.len() == 1 && lane_key(&args[0]) == lane_key(src)
        };
        if is_bcast {
            return Some(*d);
        }
    }
    None
}

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

    // 0. CROSS-SEED SPLAT CSE: a previous seed's applied plan may have
    //    emitted this exact broadcast already (the nbody pair body: the
    //    i-velocity and j-velocity seeds each splat `mag`; the fixpoint
    //    applies them one block-scan apart, so the second plan's dedup
    //    map — fresh per plan — cannot see the first). If the block
    //    already carries `broadcast(src)` (or the zero form for an
    //    all-zero constant), FORWARD its dest: zero new instructions,
    //    exactly the PackKind::Forward chaining the extract case uses.
    //    Bit-identity of the source operand is the LaneKey predicate.
    if lanes.windows(2).all(|w| lane_key(&w[0]) == lane_key(&w[1])) {
        let src0 = strip_bitidentity_casts(ctx, &lanes[0]);
        if let Some(prev) = find_existing_broadcast(ctx, fam, &src0) {
            let idx = packs.len();
            packs.push(Pack {
                kind: PackKind::Forward { val: prev },
                lane_vals: Vec::new(),
                sched: usize::MAX,
                order: 0,
            });
            dedup.insert(key, idx);
            return Some(idx);
        }
    }

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

    // 1a. SAME-SOURCE splat: lanes are VALUES that all name the same
    //     source — the same SSA value through bit-identity casts, or
    //     non-volatile loads of the same symbolic address with no
    //     intervening write that touches those bytes. The pre-CSE
    //     frontend emits one load per C spelling (`bodies[j].mass` per
    //     component of the nbody pair update), and the aliasing
    //     discipline cannot merge them across may-alias stores, so the
    //     mul trees carry N loads of ONE address — a shape the bit-exact
    //     splat can never see and the MemLoad pack rejects (the lanes
    //     are at the SAME offset, not consecutive). Broadcast lane 0's
    //     value; every other lane's def is recorded in `lane_vals` so
    //     the legality rules treat it as removed (the redundant load
    //     dies with its only consumer) while the broadcast's SOURCE is
    //     kept alive by `keep_operands`.
    //
    //     Soundness: the no-intervening-write proof is per WRITE with
    //     the full symbolic disjointness battery — same stream
    //     (base, var, mult): byte ranges; same base and mult with a
    //     different var: the FIELD-DISJOINTNESS THEOREM (the write's
    //     element window vs the loaded window — the nbody mass/vx case,
    //     where the velocity stores between the mass loads provably
    //     never touch the mass field for ANY i, j); anything else: the
    //     write rejects (conservative, exactly like rule (c)). All
    //     lanes therefore observed bit-identical bytes, and the
    //     broadcast of lane 0's read reproduces each lane's value.
    if lanes.len() >= 2 {
        let all_values: Option<Vec<&Operand>> = lanes
            .iter()
            .map(|l| match l {
                Operand::Value(_) => Some(l),
                _ => None,
            })
            .collect();
        if let Some(vals) = all_values {
            let lane0 = strip_bitidentity_casts(ctx, vals[0]);
            let lanes_same_source = vals[1..]
                .iter()
                .all(|l| same_source_loads_symbolic(ctx, &lane0, &strip_bitidentity_casts(ctx, l)));
            if lanes_same_source {
                // Cross-seed CSE for the same-source class too: an
                // earlier plan may have broadcast this exact source.
                if let Some(prev) = find_existing_broadcast(ctx, fam, &lane0) {
                    let idx = packs.len();
                    packs.push(Pack {
                        kind: PackKind::Forward { val: prev },
                        lane_vals: Vec::new(),
                        sched: usize::MAX,
                        order: 0,
                    });
                    dedup.insert(key, idx);
                    return Some(idx);
                }
                let src = lane0;
                // Remove every lane EXCEPT the broadcast source: the
                // source's def must survive (the broadcast reads it);
                // the others are dead once their consumers are packed.
                // If a lane IS the source (spelled differently), set
                // semantics make the double removal idempotent.
                let mut lane_vals: Vec<Value> = Vec::with_capacity(width);
                let mut src_id: Option<u32> = None;
                if let Operand::Value(sv) = &src {
                    src_id = Some(sv.0);
                }
                for l in vals {
                    let Operand::Value(v) = l else { continue };
                    if Some(v.0) != src_id {
                        lane_vals.push(*v);
                    }
                }
                let idx = packs.len();
                packs.push(Pack {
                    kind: PackKind::Splat {
                        src,
                        is_zero: false,
                    },
                    lane_vals,
                    sched: usize::MAX,
                    order: 0,
                });
                dedup.insert(key, idx);
                return Some(idx);
            }
        }
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
        // 1a-F. FORWARD: every lane is a lane-extract of ONE existing
        // vector at consecutive indices 0..width-1 — the pack IS that
        // vector (see `PackKind::Forward`). The extract must be THIS
        // family's own lane-extract intrinsic (an F64x2 extract
        // forwards into an F64×2 pack; an F64x4 extract has a different
        // vector width and cannot).
        if let Some(extract_op) = fam.extract {
            let extract_shape = |v: &Value| -> Option<(Value, i64)> {
                let i = ctx.def_pos.get(&v.0)?;
                match &block.instructions[*i] {
                    Instruction::Intrinsic {
                        op,
                        args,
                        dest: Some(_),
                        dest_ptr: None,
                    } if *op == extract_op && args.len() == 2 => {
                        let Operand::Value(vec) = &args[0] else {
                            return None;
                        };
                        let Operand::Const(c) = &args[1] else {
                            return None;
                        };
                        Some((*vec, c.to_i64()?))
                    }
                    _ => None,
                }
            };
            let mut forward_vec: Option<Value> = None;
            let mut forward_ok = true;
            for (li, v) in vals.iter().enumerate() {
                match extract_shape(v) {
                    Some((vec, lane)) if lane == li as i64 => match forward_vec {
                        None => forward_vec = Some(vec),
                        Some(prev) if prev == vec => {}
                        _ => {
                            forward_ok = false;
                            break;
                        }
                    },
                    _ => {
                        forward_ok = false;
                        break;
                    }
                }
            }
            if forward_ok && forward_vec.is_some() {
                let idx = packs.len();
                packs.push(Pack {
                    kind: PackKind::Forward {
                        val: forward_vec.unwrap(),
                    },
                    lane_vals: Vec::new(),
                    sched: usize::MAX,
                    order: 0,
                });
                dedup.insert(key, idx);
                return Some(idx);
            }
        }
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
            // 1b. Sub-word SELECT demotion: lanes are truncating casts of
            //     PROMOTED selects — `q[i] = a[i] < b[i] ? a[i] : b[i]` (the
            //     min/max spelling) or `? 7 : 3` (mixed arms) on i8/i16/u8/
            //     u16 arrays. C promotes the compare and BOTH arms to int;
            //     the store truncates back, so the packed replacement needs
            //     only be bit-identical to `trunc(promoted select)` — which a
            //     select at the LANE width over the raw narrow values is, by
            //     construction, for every arm constant (arms truncate
            //     exactly) and every predicate the remap table below admits.
            //     The promoted Select/Cmp chain STAYS in the IR for any wider
            //     uses (its lanes here are only the truncs), so the pack's
            //     cond_lanes are EMPTY — nothing is removed that is still
            //     used, and DCE retires the promoted chain when the truncs
            //     were its only consumers.
            {
                let sel_demotable = vals.iter().all(|v| {
                    match ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]) {
                        Some(Instruction::Cast {
                            src: Operand::Value(sv),
                            from_ty,
                            to_ty,
                            ..
                        }) if *to_ty == ty
                            && ty_is_integer(*from_ty)
                            && from_ty.size() > ty.size() =>
                        {
                            matches!(
                                ctx.def_pos.get(&sv.0).map(|&j| &block.instructions[j]),
                                Some(Instruction::Select { .. })
                            )
                        }
                        _ => false,
                    }
                });
                if sel_demotable {
                    // Per lane: (remapped compare op, stripped lhs, stripped
                    // rhs, stripped true arm, stripped false arm).
                    let mut shapes: Vec<(IrCmpOp, Operand, Operand, Operand, Operand)> =
                        Vec::with_capacity(width);
                    let mut demote_ok = true;
                    for v in &vals {
                        let i = ctx.def_pos[&v.0];
                        let Instruction::Cast {
                            src: Operand::Value(sv),
                            ..
                        } = &block.instructions[i]
                        else {
                            unreachable!()
                        };
                        let j = ctx.def_pos[&sv.0];
                        let Instruction::Select {
                            cond,
                            true_val,
                            false_val,
                            ..
                        } = &block.instructions[j]
                        else {
                            unreachable!()
                        };
                        let Operand::Value(cv) = cond else {
                            demote_ok = false;
                            break;
                        };
                        let Some(&ci) = ctx.def_pos.get(&cv.0) else {
                            demote_ok = false;
                            break;
                        };
                        let Instruction::Cmp {
                            op,
                            lhs,
                            rhs,
                            ty: cty,
                            ..
                        } = &block.instructions[ci]
                        else {
                            demote_ok = false;
                            break;
                        };
                        // The compare is either already at the lane width
                        // (an earlier pass narrowed it — the arms still carry
                        // the widening casts) or at the promoted width over
                        // widening casts/consts of the lane values.
                        let (remap, l, r) = if *cty == ty {
                            (
                                Some(*op),
                                strip_bitidentity_casts(ctx, lhs),
                                strip_bitidentity_casts(ctx, rhs),
                            )
                        } else if ty_is_integer(*cty) && cty.size() > ty.size() {
                            // Widen-cast / fitting-const operands only.
                            let strip_wide = |o: &Operand| -> Option<(bool, bool, Operand)> {
                                match o {
                                    Operand::Value(w) => {
                                        match ctx.def_pos.get(&w.0).map(|&k| &block.instructions[k])
                                        {
                                            Some(Instruction::Cast {
                                                src,
                                                from_ty,
                                                to_ty,
                                                ..
                                            }) if *from_ty == ty && *to_ty == *cty => Some((
                                                !from_ty.is_signed(),
                                                !to_ty.is_signed(),
                                                src.clone(),
                                            )),
                                            _ => None,
                                        }
                                    }
                                    Operand::Const(c) => {
                                        let cv = c.to_i64()?;
                                        // The const must compare identically after
                                        // the demotion: it has to fit the FINAL
                                        // (remapped) predicate's narrow range.
                                        // The remap here uses the SAME unsigned
                                        // flags the tuple reports (and the cast
                                        // side uses) — a signed lane promotes by
                                        // sext, so its compare stays SIGNED and
                                        // an out-of-range constant (40000 vs
                                        // int16) must reject, never truncate.
                                        // (Both-const compares fold before SLP.)
                                        let rem = demote_cmp_pred(
                                            *op,
                                            !ty.is_signed(),
                                            !cty.is_signed(),
                                        )?;
                                        let unsigned_final = matches!(
                                            rem,
                                            IrCmpOp::Ult
                                                | IrCmpOp::Ule
                                                | IrCmpOp::Ugt
                                                | IrCmpOp::Uge
                                        );
                                        if fits_narrow(cv, ty, unsigned_final) {
                                            Some((
                                                !ty.is_signed(),
                                                !cty.is_signed(),
                                                Operand::Const(narrow_const(cv, ty)),
                                            ))
                                        } else {
                                            None
                                        }
                                    }
                                }
                            };
                            match (strip_wide(lhs), strip_wide(rhs)) {
                                (Some((fu0, tu0, l)), Some((fu1, tu1, r)))
                                    if fu0 == fu1 && tu0 == tu1 =>
                                {
                                    (demote_cmp_pred(*op, fu0, tu0), l, r)
                                }
                                _ => (None, lhs.clone(), rhs.clone()),
                            }
                        } else {
                            (None, lhs.clone(), rhs.clone())
                        };
                        let Some(remapped) = remap else {
                            demote_ok = false;
                            break;
                        };
                        // The arms: widening casts of the lane values, or
                        // constants (truncation-exact for every value).
                        // The arms widen to the SELECT's type, which is the
                        // compare's type only when the compare itself is at
                        // the promoted width — an already-narrow compare
                        // (an earlier pass narrowed it) still carries
                        // full-width arms. Accept ANY wider target.
                        let strip_arm = |o: &Operand| -> Option<Operand> {
                            match o {
                                Operand::Value(w) => {
                                    match ctx.def_pos.get(&w.0).map(|&k| &block.instructions[k]) {
                                        Some(Instruction::Cast {
                                            src,
                                            from_ty,
                                            to_ty,
                                            ..
                                        }) if *from_ty == ty
                                            && ty_is_integer(*to_ty)
                                            && to_ty.size() > ty.size() =>
                                        {
                                            Some(src.clone())
                                        }
                                        _ => None,
                                    }
                                }
                                Operand::Const(c) => {
                                    c.to_i64().map(|cv| Operand::Const(narrow_const(cv, ty)))
                                }
                            }
                        };
                        let Some(t) = strip_arm(true_val) else {
                            demote_ok = false;
                            break;
                        };
                        let Some(f) = strip_arm(false_val) else {
                            demote_ok = false;
                            break;
                        };
                        shapes.push((remapped, l, r, t, f));
                    }
                    if demote_ok && shapes.len() == width {
                        let op0 = shapes[0].0;
                        let uniform = shapes.iter().all(|s| s.0 == op0);
                        if uniform {
                            // (a) the min/max spelling on the stripped
                            //     operands — same fold table as the at-width
                            //     2c path.
                            let spell = match op0 {
                                IrCmpOp::Slt | IrCmpOp::Sle => Some(0),
                                IrCmpOp::Sgt | IrCmpOp::Sge => Some(1),
                                _ => None,
                            };
                            if let Some(s) = spell {
                                #[derive(Clone, Copy, PartialEq)]
                                enum Fold {
                                    Min,
                                    Max,
                                }
                                let mut fold: Option<(Fold, bool)> = None;
                                let mut shapes_ok = true;
                                for (op, l, r, t, f) in &shapes {
                                    let _ = op;
                                    let this = if same_source(ctx, t, l) && same_source(ctx, f, r) {
                                        Some(if s == 0 {
                                            (Fold::Min, false)
                                        } else {
                                            (Fold::Max, false)
                                        })
                                    } else if same_source(ctx, t, r) && same_source(ctx, f, l) {
                                        Some(if s == 0 {
                                            (Fold::Max, true)
                                        } else {
                                            (Fold::Min, true)
                                        })
                                    } else {
                                        None
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
                                if shapes_ok {
                                    if let Some((kind, arms_swapped)) = fold {
                                        if let Some(vec_op) =
                                            packed_int_minmax(kind == Fold::Max, ty, width)
                                        {
                                            let mut src1: Vec<Operand> = Vec::with_capacity(width);
                                            let mut src2: Vec<Operand> = Vec::with_capacity(width);
                                            for (_, l, r, _, _) in &shapes {
                                                if arms_swapped {
                                                    src1.push(strip_bitidentity_casts(ctx, r));
                                                    src2.push(strip_bitidentity_casts(ctx, l));
                                                } else {
                                                    src1.push(strip_bitidentity_casts(ctx, l));
                                                    src2.push(strip_bitidentity_casts(ctx, r));
                                                }
                                            }
                                            if let (Some(lhs), Some(rhs)) = (
                                                build_pack(
                                                    ctx,
                                                    &src1,
                                                    ty,
                                                    width,
                                                    fam,
                                                    packs,
                                                    dedup,
                                                    depth + 1,
                                                ),
                                                build_pack(
                                                    ctx,
                                                    &src2,
                                                    ty,
                                                    width,
                                                    fam,
                                                    packs,
                                                    dedup,
                                                    depth + 1,
                                                ),
                                            ) {
                                                let idx = packs.len();
                                                packs.push(Pack {
                                                    kind: PackKind::FpMinMax {
                                                        vec_op,
                                                        lhs,
                                                        rhs,
                                                        // The promoted Cmp stays
                                                        // (the promoted Select still
                                                        // reads it); nothing is
                                                        // removed on its behalf.
                                                        cond_lanes: Vec::new(),
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
                            // (b) the general cmp+blendv composite with the
                            //     remapped predicate.
                            if let Some((pred, swap)) = cmp_predicate_imm(op0, false) {
                                if let Some((cmp_op, blend_op)) = packed_cmp_blendv(ty, width) {
                                    let mut a_lanes: Vec<Operand> = Vec::with_capacity(width);
                                    let mut b_lanes: Vec<Operand> = Vec::with_capacity(width);
                                    let mut t_lanes: Vec<Operand> = Vec::with_capacity(width);
                                    let mut f_lanes: Vec<Operand> = Vec::with_capacity(width);
                                    for (_, l, r, t, f) in &shapes {
                                        // The mirrored relations compare (r, l).
                                        if swap {
                                            a_lanes.push(r.clone());
                                            b_lanes.push(l.clone());
                                        } else {
                                            a_lanes.push(l.clone());
                                            b_lanes.push(r.clone());
                                        }
                                        t_lanes.push(t.clone());
                                        f_lanes.push(f.clone());
                                    }
                                    if let (Some(lhs), Some(rhs), Some(tv), Some(fv)) = (
                                        build_pack(
                                            ctx,
                                            &a_lanes,
                                            ty,
                                            width,
                                            fam,
                                            packs,
                                            dedup,
                                            depth + 1,
                                        ),
                                        build_pack(
                                            ctx,
                                            &b_lanes,
                                            ty,
                                            width,
                                            fam,
                                            packs,
                                            dedup,
                                            depth + 1,
                                        ),
                                        build_pack(
                                            ctx,
                                            &t_lanes,
                                            ty,
                                            width,
                                            fam,
                                            packs,
                                            dedup,
                                            depth + 1,
                                        ),
                                        build_pack(
                                            ctx,
                                            &f_lanes,
                                            ty,
                                            width,
                                            fam,
                                            packs,
                                            dedup,
                                            depth + 1,
                                        ),
                                    ) {
                                        let idx = packs.len();
                                        packs.push(Pack {
                                            kind: PackKind::CmpBlendv {
                                                cmp_op,
                                                blend_op,
                                                pred,
                                                lhs,
                                                rhs,
                                                tv,
                                                fv,
                                                // Empty: the promoted Cmp is
                                                // not removed (the surviving
                                                // promoted Select reads it).
                                                cond_lanes: Vec::new(),
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
                // 2a. STREAM CSE: the scalar IR materializes ONE Load per
                // use site, so the same address window arrives here once
                // per consumer (the rotate composite's five separate
                // m[i-2] loads — its same-source proof accepts
                // same-address loads by symbolic evaluation). A prior
                // MemLoad pack of this plan with the SAME stream
                // (base, var, mult), the SAME first offset and the SAME
                // lane count reads exactly the same bytes — PROVIDED no
                // memory write intervenes between the two scalar load
                // groups (a write there would mean the scalar code
                // itself read two different values, and forwarding would
                // silently pick one). Fail-closed: any Store in the
                // position window between the two groups blocks the CSE.
                // This is GCC's schedule form: each pair loads ONCE.
                let a0 = &addrs[0];
                if let Some(prior) = packs.iter().position(|p| {
                    let PackKind::MemLoad {
                        ptrs: pp, offs: po, ..
                    } = &p.kind
                    else {
                        return false;
                    };
                    if po.len() != width || po.first() != Some(&a0.off) {
                        return false;
                    }
                    // WO-7 (red-team audit, 2026-09-18): pin the
                    // construction-time invariant AT THE POINT OF USE. The
                    // CSE's "same stream + same first offset + same width ⇒
                    // identical bytes" reasoning additionally needs `po` to
                    // be a strict arithmetic progression at the family
                    // stride; MemLoad construction guarantees it (the
                    // `addrs.windows(2)` exact-delta check above), and this
                    // assert makes a future construction change fail loudly
                    // here instead of silently breaking the equivalence.
                    debug_assert!(po.iter().enumerate().all(|(i, &o)| {
                        o == po.first().copied().unwrap() + i as i64 * fam.size as i64
                    }));
                    match eval_sym_addr(block, &ctx.def_pos, pp[0]) {
                        Some(pa)
                            if pa.base == a0.base && pa.var == a0.var && pa.mult == a0.mult =>
                        {
                            // Same stream and window: now the no-write
                            // interval spanning BOTH groups' reads —
                            // the earliest to the latest position of
                            // either group's lanes.
                            let prior_pos: Vec<usize> = p
                                .lane_vals
                                .iter()
                                .filter_map(|v| ctx.def_pos.get(&v.0).copied())
                                .collect();
                            let this_pos: Vec<usize> = vals
                                .iter()
                                .filter_map(|v| ctx.def_pos.get(&v.0).copied())
                                .collect();
                            let lo = prior_pos
                                .iter()
                                .chain(this_pos.iter())
                                .copied()
                                .min()
                                .unwrap_or(usize::MAX);
                            let hi = prior_pos
                                .iter()
                                .chain(this_pos.iter())
                                .copied()
                                .max()
                                .unwrap_or(usize::MAX);
                            // A memory write in the window blocks the CSE
                            // UNLESS it is provably disjoint from the
                            // loaded bytes: same symbolic stream with
                            // non-overlapping byte ranges (the schedule's
                            // seed stores at +0/+4 vs the rotate windows
                            // at −8/−4 — disjoint by the exact affine
                            // offsets), exactly the byte-precision
                            // discipline of rules (c)/(d)/(e). Different
                            // or opaque streams block conservatively.
                            let window_lo = a0.off as i128;
                            let window_hi = window_lo + width as i128 * fam.size as i128;
                            !(lo..=hi).any(|q| {
                                let inst = &block.instructions[q];
                                let is_write = matches!(
                                    inst,
                                    Instruction::Store { .. }
                                        | Instruction::AtomicStore { .. }
                                        | Instruction::AtomicRmw { .. }
                                        | Instruction::AtomicCmpxchg { .. }
                                        | Instruction::Call { .. }
                                        | Instruction::CallIndirect { .. }
                                        | Instruction::InlineAsm { .. }
                                        | Instruction::DynAlloca { .. }
                                        | Instruction::Memcpy { .. }
                                );
                                if !is_write {
                                    return false;
                                }
                                if let Instruction::Store { ptr, ty: sty, .. } = inst {
                                    if let Some(sa) = eval_sym_addr(block, &ctx.def_pos, *ptr) {
                                        if sa.base == a0.base
                                            && sa.var == a0.var
                                            && sa.mult == a0.mult
                                        {
                                            let s_lo = sa.off as i128;
                                            let s_hi = s_lo + sty.size() as i128;
                                            return s_lo < window_hi && window_lo < s_hi;
                                        }
                                        // Different stream: provably
                                        // disjoint only via base identity
                                        // (the RestrictBases machinery is
                                        // not threaded here) — block.
                                        return true;
                                    }
                                }
                                true
                            })
                        }
                        _ => false,
                    }
                }) {
                    return Some(prior);
                }
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

        // 2b-FP. FP Negation lanes (UnaryOp{Neg, F32/F64} at the seed's
        //       lane type): the composite `Neg(x) == Xor(x, -0.0)`.
        //       Bit-exactness PROOF against the scalar lowering: the x86
        //       scalar path (alu.rs `emit_float_neg_impl`) XORs the value's
        //       GPR bit pattern with the sign mask — a pure sign-bit flip,
        //       NOT a `0.0 - x` subtraction — so the packed `vxorps/vxorpd`
        //       with the per-lane sign mask produces the identical lane
        //       bits for EVERY value: normals, subnormals, infinities,
        //       ±0 (the SIGN flips — exactly what unary minus means for
        //       zeros), and every NaN payload (flipped sign, payload
        //       preserved verbatim, no canonicalisation).
        //       Emitted as ONE intrinsic with a Const(-0.0) second operand:
        //       the backend folds the mask as a .rodata memory operand —
        //       one instruction per pack, the exact GCC spelling.
        if matches!(ty, IrType::F32 | IrType::F64) {
            let neg_lanes_ok = vals.iter().all(|v| {
                matches!(
                    ctx.def_pos.get(&v.0).map(|&i| &block.instructions[i]),
                    Some(Instruction::UnaryOp {
                        op: IrUnaryOp::Neg,
                        ty: uty,
                        ..
                    }) if *uty == ty
                )
            });
            if neg_lanes_ok {
                if let Some(vec_op) = packed_fpxor(ty, width) {
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
                    if let Some(val) =
                        build_pack(ctx, &srcs, ty, width, fam, packs, dedup, depth + 1)
                    {
                        let idx = packs.len();
                        packs.push(Pack {
                            kind: PackKind::FpNeg { vec_op, val },
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

        // 2c. Select-of-Cmp lanes: every lane is
        //     `Select(cond = Cmp(op, l, r), t, f)` with the compare at the
        //     seed's lane type. Two folds, tried in order:
        //     (a) MIN/MAX — the same-source-arm spellings. FP: the four
        //         STRICT-ORDERED forms (the FpMinMax lane-exactness proof:
        //         MINPS/MAXPS return src2 on unordered/both-zero, exactly
        //         the false arm). INTEGER: every relational spelling is
        //         exact (integer min/max is commutative with no NaN/±0
        //         asymmetry, and `<=`/`>=` differ from `<`/`>` only on
        //         equal values — where both arms ARE the same value).
        //     (b) CMP+BLENDV — everything else (mixed arms, non-foldable
        //         predicates): one packed compare producing an all-ones /
        //         all-zeros lane mask (IEEE-exact for every FP predicate —
        //         LT_OS is exactly C `<`, NEQ_UQ exactly C `!=`, all false
        //         on NaN just like C) + one BITWISE lane select. Exact for
        //         every predicate, every arm pair, integer and FP.
        //     Operand identity in (a) is compared BIT-EXACTLY (`lane_key`),
        //     never by `Operand`'s derived float equality — a {-0.0, +0.0}
        //     mismatch under `==` would fold a ternary whose ±0 lanes the
        //     packed op answers differently.
        {
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
                    if *cty != ty
                        && !(ty_is_integer(*cty) && ty_is_integer(ty) && cty.size() == ty.size())
                    {
                        // The compare must sit at the pack's lane type,
                        // or at a SAME-WIDTH integer sibling — a
                        // `(int32_t)uint32_t` reinterpret whose bits the
                        // predicate (which carries the signedness itself)
                        // reads identically. FP lanes never reinterpret.
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
                let is_fp = matches!(ty, IrType::F32 | IrType::F64);
                // ── (a) the min/max fold ──────────────────────────────
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
                    // FP admits only the STRICT spellings (non-strict
                    // differs on ±0); integer lanes admit every relational
                    // spelling (exactness argument above).
                    let spell = match op {
                        IrCmpOp::Slt | IrCmpOp::Sle => Some(0),
                        IrCmpOp::Sgt | IrCmpOp::Sge => Some(1),
                        _ => None,
                    };
                    let this = match (spell, is_fp && matches!(op, IrCmpOp::Sle | IrCmpOp::Sge)) {
                        (Some(s), false) => {
                            if same_source(ctx, &t, &l) && same_source(ctx, &f, &r) {
                                Some(if s == 0 {
                                    (Fold::Min, false)
                                } else {
                                    (Fold::Max, false)
                                })
                            } else if same_source(ctx, &t, &r) && same_source(ctx, &f, &l) {
                                Some(if s == 0 {
                                    (Fold::Max, true)
                                } else {
                                    (Fold::Min, true)
                                })
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
                if shapes_ok && fold.is_some() {
                    let (kind, arms_swapped) = fold.unwrap();
                    let vec_op = if is_fp {
                        packed_minmax(kind == Fold::Max, ty, width)
                    } else {
                        packed_int_minmax(kind == Fold::Max, ty, width)
                    };
                    if let Some(vec_op) = vec_op {
                        // src1/src2 lanes per the fold's operand order:
                        // arms_swapped selects (r, l) for the mirrored
                        // spellings — the FALSE arm must be src2 (the FP
                        // operand contract; integer min/max is commutative
                        // so either order is exact there).
                        let mut src1: Vec<Operand> = Vec::with_capacity(width);
                        let mut src2: Vec<Operand> = Vec::with_capacity(width);
                        let mut conds: Vec<Value> = Vec::with_capacity(width);
                        for v in &vals {
                            let (_, l, r, _, _, cv) = sel_shape(v).unwrap();
                            if arms_swapped {
                                src1.push(strip_bitidentity_casts(ctx, &r));
                                src2.push(strip_bitidentity_casts(ctx, &l));
                            } else {
                                src1.push(strip_bitidentity_casts(ctx, &l));
                                src2.push(strip_bitidentity_casts(ctx, &r));
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
                // ── (b) the general cmp+blendv fold ────────────────────
                // Uniform compare op across lanes; the predicate immediate
                // follows the VecCmp* vocabulary (mirrored relations swap
                // the compare operands and use the primary predicate).
                // FP admits the six C-spelling predicates; unsigned
                // relations only exist for integer lanes.
                if let Some(first) = vals.first() {
                    let (op0, _, _, _, _, _) = sel_shape(first).unwrap();
                    let pred = cmp_predicate_imm(op0, is_fp);
                    if let Some((pred, swap)) = pred {
                        if let Some((cmp_op, blend_op)) = packed_cmp_blendv(ty, width) {
                            let mut l_lanes: Vec<Operand> = Vec::with_capacity(width);
                            let mut r_lanes: Vec<Operand> = Vec::with_capacity(width);
                            let mut t_lanes: Vec<Operand> = Vec::with_capacity(width);
                            let mut f_lanes: Vec<Operand> = Vec::with_capacity(width);
                            let mut conds: Vec<Value> = Vec::with_capacity(width);
                            let mut uniform = true;
                            for v in &vals {
                                let (op, l, r, t, f, cv) = sel_shape(v).unwrap();
                                if op != op0 {
                                    uniform = false;
                                    break;
                                }
                                l_lanes.push(strip_bitidentity_casts(ctx, &l));
                                r_lanes.push(strip_bitidentity_casts(ctx, &r));
                                t_lanes.push(t.clone());
                                f_lanes.push(f.clone());
                                conds.push(cv);
                            }
                            if uniform {
                                // The mirrored relations compare (r, l).
                                let (a_lanes, b_lanes) = if swap {
                                    (&r_lanes, &l_lanes)
                                } else {
                                    (&l_lanes, &r_lanes)
                                };
                                if let (Some(lhs), Some(rhs), Some(tv), Some(fv)) = (
                                    build_pack(
                                        ctx,
                                        a_lanes,
                                        ty,
                                        width,
                                        fam,
                                        packs,
                                        dedup,
                                        depth + 1,
                                    ),
                                    build_pack(
                                        ctx,
                                        b_lanes,
                                        ty,
                                        width,
                                        fam,
                                        packs,
                                        dedup,
                                        depth + 1,
                                    ),
                                    build_pack(
                                        ctx,
                                        &t_lanes,
                                        ty,
                                        width,
                                        fam,
                                        packs,
                                        dedup,
                                        depth + 1,
                                    ),
                                    build_pack(
                                        ctx,
                                        &f_lanes,
                                        ty,
                                        width,
                                        fam,
                                        packs,
                                        dedup,
                                        depth + 1,
                                    ),
                                ) {
                                    let idx = packs.len();
                                    packs.push(Pack {
                                        kind: PackKind::CmpBlendv {
                                            cmp_op,
                                            blend_op,
                                            pred,
                                            lhs,
                                            rhs,
                                            tv,
                                            fv,
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
                // 3f. PACKED FMA CONTRACTION — tried FIRST among the
                //     Add/Sub shapes (strictly better when it applies:
                //     one instruction where the generic path builds a
                //     Mul pack + an Add/Sub pack, AND the rounding
                //     parity the scalar gap-fused contraction already
                //     guarantees — the tri-config differential would
                //     otherwise drift by the extra multiply rounding).
                //     Lanes: `acc ± Mul(x, s)` with the s side UNIFORM
                //     across lanes (bit-identical lane_keys or
                //     same-source loads — exactly the Splat pack's
                //     domain), the acc side any packable operand run
                //     (the MemLoad accumulator of a load-modify-store,
                //     a forwarded vector, ...). FMA3 + the FP contract
                //     gate the whole shape; the emitted intrinsic
                //     (VecFma/VecFnma {F64x2,F32x4}) mirrors the scalar
                //     emit_fused_mul_{add,sub} rounding discipline.
                if matches!(op, IrBinOp::Add | IrBinOp::Sub)
                    && matches!(ty, IrType::F64 | IrType::F32)
                    && crate::passes::vectorize::x86_fma_available_pub()
                    && ctx.fp_contract != crate::common::fp_contract::FpContract::Off
                {
                    // Decompose each lane: which side is the Mul, which
                    // the accumulator. The orientation must be uniform
                    // across lanes.
                    let mul_side = |o: &Operand| -> Option<(Operand, Operand)> {
                        let Operand::Value(mv) = o else {
                            return None;
                        };
                        let mi = ctx.def_pos.get(&mv.0)?;
                        match &block.instructions[*mi] {
                            Instruction::BinOp {
                                op: IrBinOp::Mul,
                                lhs,
                                rhs,
                                ty: mty,
                                ..
                            } if *mty == ty => Some((lhs.clone(), rhs.clone())),
                            _ => None,
                        }
                    };
                    let mut acc_lanes: Vec<Operand> = Vec::with_capacity(width);
                    let mut mul_lhs_lanes: Vec<Operand> = Vec::with_capacity(width);
                    let mut mul_rhs_lanes: Vec<Operand> = Vec::with_capacity(width);
                    let mut orientation: Option<bool> = None; // true: mul on lhs
                    let mut fma_ok = true;
                    for v in &vals {
                        let i = ctx.def_pos[&v.0];
                        let Instruction::BinOp { lhs, rhs, .. } = &block.instructions[i] else {
                            unreachable!()
                        };
                        let (mul_on_lhs, mul_ops, acc) = match (mul_side(lhs), mul_side(rhs)) {
                            (Some(m), None) => (true, m, rhs.clone()),
                            (None, Some(m)) => (false, m, lhs.clone()),
                            _ => {
                                fma_ok = false;
                                break;
                            }
                        };
                        match orientation {
                            None => orientation = Some(mul_on_lhs),
                            Some(o) if o == mul_on_lhs => {}
                            _ => {
                                fma_ok = false;
                                break;
                            }
                        }
                        acc_lanes.push(acc);
                        mul_lhs_lanes.push(mul_ops.0);
                        mul_rhs_lanes.push(mul_ops.1);
                    }
                    if fma_ok && orientation.is_some() {
                        // Which mul side is uniform (the splat b)?
                        // Bit-identical lane_keys first, then the
                        // same-source proof (the mass-load class).
                        let uniform_side = |lanes: &[Operand]| -> bool {
                            if lanes.is_empty() {
                                return false;
                            }
                            let l0 = strip_bitidentity_casts(ctx, &lanes[0]);
                            lanes[1..].iter().all(|l| {
                                let ls = strip_bitidentity_casts(ctx, l);
                                lane_key(&l0) == lane_key(&ls)
                                    || same_source_loads_symbolic(ctx, &l0, &ls)
                            })
                        };
                        let (x_lanes, s_lanes) = if uniform_side(&mul_lhs_lanes) {
                            (mul_rhs_lanes.clone(), mul_lhs_lanes.clone())
                        } else if uniform_side(&mul_rhs_lanes) {
                            (mul_lhs_lanes.clone(), mul_rhs_lanes.clone())
                        } else {
                            (Vec::new(), Vec::new())
                        };
                        if !x_lanes.is_empty() {
                            if let (Some(acc_p), Some(a_p), Some(b_p)) = (
                                build_pack(
                                    ctx,
                                    &acc_lanes,
                                    ty,
                                    width,
                                    fam,
                                    packs,
                                    dedup,
                                    depth + 1,
                                ),
                                build_pack(ctx, &x_lanes, ty, width, fam, packs, dedup, depth + 1),
                                build_pack(ctx, &s_lanes, ty, width, fam, packs, dedup, depth + 1),
                            ) {
                                // The splat side must actually BE a splat
                                // (a gather would re-materialise the
                                // uniform value per lane — the generic
                                // path is then no worse).
                                // The uniform side must actually BE a
                                // broadcast-shaped leaf: a Splat, or a
                                // FORWARD of an existing broadcast of
                                // exactly this uniform source (the
                                // cross-seed CSE). A gather re-materialises
                                // the uniform value per lane (the generic
                                // path is then no worse); a Forward of a
                                // NON-broadcast vector (the (dx,dy)
                                // difference) is not uniform and must
                                // reject.
                                let b_is_splat = match &packs[b_p].kind {
                                    PackKind::Splat { .. } => true,
                                    PackKind::Forward { val } => {
                                        let s0 = strip_bitidentity_casts(ctx, &s_lanes[0]);
                                        find_existing_broadcast(ctx, fam, &s0)
                                            .is_some_and(|prev| prev == *val)
                                    }
                                    _ => false,
                                };
                                if b_is_splat {
                                    let idx = packs.len();
                                    packs.push(Pack {
                                        kind: PackKind::Fma {
                                            negate: op == IrBinOp::Sub,
                                            acc: acc_p,
                                            a: a_p,
                                            b: b_p,
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
                PackKind::Fma { acc, a, b, .. } => {
                    stack.push(acc);
                    stack.push(a);
                    stack.push(b);
                }
                PackKind::ShiftImm { val, .. } => {
                    stack.push(val);
                }
                PackKind::FpMinMax { lhs, rhs, .. } => {
                    stack.push(lhs);
                    stack.push(rhs);
                }
                PackKind::FpNeg { val, .. } => {
                    stack.push(val);
                }
                PackKind::CmpBlendv {
                    lhs, rhs, tv, fv, ..
                } => {
                    stack.push(lhs);
                    stack.push(rhs);
                    stack.push(tv);
                    stack.push(fv);
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
                    PackKind::Fma { acc, a, b, .. } => {
                        *acc = remap[*acc];
                        *a = remap[*a];
                        *b = remap[*b];
                    }
                    PackKind::ShiftImm { val, .. } => {
                        *val = remap[*val];
                    }
                    PackKind::FpMinMax { lhs, rhs, .. } => {
                        *lhs = remap[*lhs];
                        *rhs = remap[*rhs];
                    }
                    PackKind::FpNeg { val, .. } => {
                        *val = remap[*val];
                    }
                    PackKind::CmpBlendv {
                        lhs, rhs, tv, fv, ..
                    } => {
                        *lhs = remap[*lhs];
                        *rhs = remap[*rhs];
                        *tv = remap[*tv];
                        *fv = remap[*fv];
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
                    // An Fma consumes its accumulator and both
                    // multiplicand packs.
                    PackKind::Fma { acc, a, b, .. } => Some(vec![*acc, *a, *b]),
                    // A ShiftImm consumes its operand pack the same way a
                    // BinOp consumes its sides.
                    PackKind::ShiftImm { val, .. } => Some(vec![*val]),
                    PackKind::FpMinMax { lhs, rhs, .. } => Some(vec![*lhs, *rhs]),
                    PackKind::FpNeg { val, .. } => Some(vec![*val]),
                    PackKind::CmpBlendv {
                        lhs, rhs, tv, fv, ..
                    } => Some(vec![*lhs, *rhs, *tv, *fv]),
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
        if let PackKind::FpMinMax { cond_lanes, .. } | PackKind::CmpBlendv { cond_lanes, .. } =
            &p.kind
        {
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
            // Select and Cmp join the feeder kinds for the sub-word
            // SELECT demotion: the promoted chain (Select reading a Cmp
            // and widening Casts, feeding only the removed truncating
            // lanes) is exactly the scaffolding that must retire here —
            // rule (a) would otherwise see its reads of the load lanes
            // as live in-block uses at or before the pack slot.
            if !matches!(
                inst,
                Instruction::Cast { .. }
                    | Instruction::BinOp { .. }
                    | Instruction::Select { .. }
                    | Instruction::Cmp { .. }
            ) {
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
            // (b) CROSS-BLOCK USES ARE PERMITTED (v6 relaxation of the
            // original blanket rejection). Soundness PROOF: the extract
            // is placed in the lane's defining block B at p.sched — the
            // maximum of the pack's lane-def positions, i.e. AFTER every
            // lane def and before B's terminator. For ANY use of the
            // lane (an instruction in another block U, or a phi incoming
            // on an edge P→U), SSA validity gives def(v) dom use; block
            // linearity means any path containing def(v) executes all
            // of B — including the later extract position — before
            // leaving B, so the extract dominates the same use. The
            // rewrite (apply_plan rewrites EVERY block's uses) replaces
            // the use with the extract value; nothing reads a deleted
            // def.
            //
            // The one EXCEPTION kept from the original rule: in-block
            // uses BEFORE the pack's slot (rule (a) below) — including
            // self-loop backedge phi incomings recorded at the phi's
            // position.
            //
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
        // The min/max and cmp+blendv folds' Cmp lanes: NO extract exists
        // for them (a scalar bool cannot be reconstructed from the packed
        // mask), so EVERY use outside the removed set — early OR late,
        // terminator included — rejects the seed. (The early-only variant
        // left the cmp's later consumers reading a deleted def: backend
        // ICE.)
        if let PackKind::FpMinMax { cond_lanes, .. } | PackKind::CmpBlendv { cond_lanes, .. } =
            &p.kind
        {
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
        // (c) No memory write strictly between a lane's load and the
        // pack's vector load position that touches THAT LANE's bytes:
        // the vector load re-reads every lane at M = max lane def, so a
        // write in (P_k, M) overlapping lane k's byte range would change
        // what lane k observes. PER-LANE precision (the v8 refinement):
        // the original whole-window test [off0, off0+width·size)
        // rejected a write touching ANY lane — even one whose load sits
        // after the write (nothing observed changes for it) or whose
        // own bytes the write misses. The per-lane byte ranges make
        // interleaved SAME-OBJECT field traffic legal exactly when it
        // is observationally inert.
        // ESCAPES (the disjointness battery): same stream
        // (base, var, mult) → exact byte ranges; same base and stride
        // with a different index variable → the FIELD-DISJOINTNESS
        // THEOREM (struct-field traffic across i/j indices); otherwise
        // the restrict contract / global / alloca object identity.
        // Anything not a plain Store (calls, atomics, asm, vector
        // intrinsics) keeps the conservative rejection.
        if let PackKind::MemLoad { ptrs, offs, .. } = &p.kind {
            let positions: Vec<usize> = p.lane_vals.iter().map(|v| ctx.def_pos[&v.0]).collect();
            let hi = *positions.iter().max().unwrap();
            let lane_addr = eval_sym_addr(block, &ctx.def_pos, ptrs[0]);
            for (li, &v) in p.lane_vals.iter().enumerate() {
                let pk = positions[li];
                let lane_off = offs[li];
                for q in pk + 1..hi {
                    if !is_memory_write(&block.instructions[q]) || removed.contains(&q) {
                        continue;
                    }
                    let (ptr, acc_size) = match &block.instructions[q] {
                        Instruction::Store { ptr, ty, .. } => (*ptr, ty.size() as i128),
                        _ => return None,
                    };
                    let touches_lane = match (&lane_addr, eval_sym_addr(block, &ctx.def_pos, ptr)) {
                        (Some(la), Some(wa))
                            if la.base == wa.base && la.var == wa.var && la.mult == wa.mult =>
                        {
                            byte_ranges_overlap(lane_off, fam.size as i64, wa.off, acc_size as i64)
                        }
                        (Some(la), Some(wa)) if la.base == wa.base && la.mult == wa.mult => {
                            !field_disjoint(
                                la.mult,
                                lane_off,
                                fam.size as i64,
                                wa.off,
                                acc_size as i64,
                            )
                        }
                        (Some(la), Some(wa)) => !bases.disjoint(la.base.0, wa.base.0),
                        _ => true,
                    };
                    if touches_lane {
                        return None;
                    }
                }
            }
        }
    }
    // (d) No memory access between a lane's OWN seed store and the batched
    // vector-store commit that touches THAT LANE's bytes. The vector store
    // commits every lane at m_max, so for lane k (scalar store at s_k):
    //   * a READ at q with s_k < q < m_max originally observed lane k's
    //     POST-store value but reads the PRE-commit value after the
    //     rewrite — miscompile;
    //   * a WRITE at q with s_k < q < m_max originally had its effect
    //     overwritten by nothing (s_k already committed) — the scalar
    //     final state is the write's value — but the rewrite's commit at
    //     m_max clobbers it — miscompile;
    //   * any access at q ≤ s_k is inert for lane k (reads see the same
    //     pre-store bytes; writes are overwritten by s_k's own commit
    //     either way).
    // PER-LANE precision (the v8 refinement of the original whole-window
    // test): only accesses after that lane's own store, overlapping that
    // lane's byte range, reject. ESCAPES (the disjointness battery):
    // same stream → exact byte ranges; same base and stride with a
    // different index variable → the FIELD-DISJOINTNESS THEOREM;
    // otherwise restrict/global/alloca object identity. Everything
    // unanalyzable (calls, atomics, asm, vector intrinsics, opaque
    // pointers) keeps the conservative rejection.
    let m_min = *cand.store_idx.iter().min().unwrap();
    let m_max = *cand.store_idx.iter().max().unwrap();
    let seed_addr = eval_sym_addr(block, &ctx.def_pos, cand.anchor_ptr);
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
        let access_addr = eval_sym_addr(block, &ctx.def_pos, ptr);
        for (li, &s_k) in cand.store_idx.iter().enumerate() {
            if s_k >= q {
                continue; // lane k's store has not committed yet at q
            }
            let lane_off = seed_addr
                .as_ref()
                .map(|sa| sa.off + li as i64 * fam.size as i64)
                .unwrap_or(0);
            let touches_lane = match (&seed_addr, &access_addr) {
                (Some(sa), Some(aa))
                    if sa.base == aa.base && sa.var == aa.var && sa.mult == aa.mult =>
                {
                    byte_ranges_overlap(lane_off, fam.size as i64, aa.off, acc_size as i64)
                }
                (Some(sa), Some(aa)) if sa.base == aa.base && sa.mult == aa.mult => {
                    !field_disjoint(sa.mult, lane_off, fam.size as i64, aa.off, acc_size as i64)
                }
                (Some(sa), Some(aa)) => !bases.disjoint(sa.base.0, aa.base.0),
                _ => true,
            };
            if touches_lane {
                return None;
            }
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
                        // FIELD-DISJOINTNESS THEOREM: same base and byte
                        // stride with a DIFFERENT index variable — the
                        // store's element window vs the lane's window can
                        // never overlap for ANY index values (the struct-
                        // field traffic the whole-window test had to
                        // reject because the streams carry different
                        // vars).
                        (Some(sa), Some((lb_base, _, lb_mult)))
                            if sa.base == *lb_base && sa.mult == *lb_mult =>
                        {
                            let store_off = sa.off + si as i64 * fam.size as i64;
                            !field_disjoint(
                                sa.mult,
                                store_off,
                                fam.size as i64,
                                offs[li],
                                fam.size as i64,
                            )
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
                .unwrap_or(false)
                // Cross-block uses (rule (b)'s relaxation) need extracts
                // exactly like surviving in-block uses: the extract lives
                // with the pack and every rewritten external use reads it.
                || ctx.external_uses.contains(&v.0);
            if has_external {
                // Only families with an exact lane-extract intrinsic can
                // service external uses.
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
            // A forwarded vector replaces W scalar operand lanes with
            // ZERO new instructions — the full scalar-op saving with no
            // vector-op cost (the vector already exists).
            PackKind::Forward { .. } => {
                benefit += width as i64;
            }
            // The FMA contraction replaces W muls AND W adds/subs with ONE
            // packed op — the same 2(W−1) the Mul+Add pair would count —
            // and removes the rounding drift besides.
            PackKind::Fma { .. } => {
                benefit += 2 * (width as i64 - 1);
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
            // FP negation: W scalar negs → ONE packed XOR whose sign mask
            // is a free .rodata memory operand (no splat, no register).
            PackKind::FpNeg { .. } => {
                benefit += width as i64 - 1;
            }
            // The cmp+blendv composite: W scalar compares AND W selects
            // → TWO packed ops (the mask is exact for every predicate).
            PackKind::CmpBlendv { .. } => {
                benefit += 2 * (width as i64 - 1);
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
        PackKind::Fma { acc, a, b, .. } => {
            1 + topo_depth(packs, *acc)
                .max(topo_depth(packs, *a))
                .max(topo_depth(packs, *b))
        }
        PackKind::ShiftImm { val, .. } => 1 + topo_depth(packs, *val),
        PackKind::FpNeg { val, .. } => 1 + topo_depth(packs, *val),
        PackKind::CmpBlendv {
            lhs, rhs, tv, fv, ..
        } => {
            1 + topo_depth(packs, *lhs)
                .max(topo_depth(packs, *rhs))
                .max(topo_depth(packs, *tv))
                .max(topo_depth(packs, *fv))
        }
        PackKind::FpMinMax { lhs, rhs, .. } => {
            1 + topo_depth(packs, *lhs).max(topo_depth(packs, *rhs))
        }
        _ => 0,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Rewrite
// ─────────────────────────────────────────────────────────────────────────

/// Look up an index variable's integer type from its defining instruction
/// anywhere in the function (the SIB decomposition's `var` is typically a
/// LOOP-HEADER phi — the body block under SLP only uses it). SSA
/// guarantees the def dominates every use, so materializing arithmetic
/// over it at position 0 of the using block is well-formed. `None` for
/// values with no integer def in this function (opaque foreign vars) —
/// the caller keeps the materialized pointer, fail-closed.
fn lookup_var_ty(func: &IrFunction, v: Value) -> Option<IrType> {
    func.blocks
        .iter()
        .find_map(|blk| {
            blk.instructions.iter().find_map(|inst| {
                let d = inst.dest()?;
                if d.0 != v.0 {
                    return None;
                }
                match inst {
                    Instruction::Phi { ty, .. } => Some(*ty),
                    Instruction::ParamRef { ty, .. } => Some(*ty),
                    Instruction::Cast { to_ty, .. } => Some(*to_ty),
                    Instruction::BinOp { ty, .. } => Some(*ty),
                    Instruction::Load { ty, .. } => Some(*ty),
                    _ => None,
                }
            })
        })
        .filter(|t| t.is_integer() && t.size() <= 8)
}

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
    // The CmpBlendv packs carry TWO vector results (the compare mask and
    // the blend output); the mask value gets its own SSA id here.
    let mut cmp_mask_dest: FxHashMap<usize, Value> = FxHashMap::default();
    for (pi, p) in plan.packs.iter().enumerate() {
        if matches!(p.kind, PackKind::CmpBlendv { .. }) {
            cmp_mask_dest.insert(pi, Value(func.next_value_id));
            func.next_value_id += 1;
        }
    }
    let mut extract_dest: FxHashMap<u32, Value> = FxHashMap::default();
    for (&vid, _) in &plan.extract_of {
        extract_dest.insert(vid, Value(func.next_value_id));
        func.next_value_id += 1;
    }
    // FORWARD packs emit no instruction: their vector result IS the
    // forwarded value (the pre-wasted fresh id above is simply unused).
    // Overriding `vec_dest` here makes every consumer's
    // `Operand::Value(vec_dest[idx])` reference the existing vector.
    for (pi, p) in plan.packs.iter().enumerate() {
        if let PackKind::Forward { val } = &p.kind {
            vec_dest[pi] = *val;
        }
    }

    // ── SIB-form addressing for index-variable streams ───────────────────
    //
    // A MemLoad stream whose symbolic address carries an index variable
    // (base + var·mult + off) currently names the MATERIALIZED pointer
    // (the scalar GEP chain): the unrolled sha256 schedule paid FOUR
    // leal+shlq+leaq chains per group — 12 instructions to restate
    // `m + (i−c)·4` four times. The x86 back end's `vec_mem_operand`
    // folds `(base, index, disp)` into one SIB operand
    // `disp(%base,%index)`, so ONE shared `idx = cast(var) << k` per
    // (var, mult) plus constant displacements replaces every chain
    // (GCC's exact schedule-loop form: one advancing index, folded
    // taps). The displaced GEP chains lose their only consumers and die
    // in the next DCE sweep.
    //
    // PLACEMENT: the materialization must respect INTRA-BLOCK SSA order
    // — a var defined in THIS block (a computed index in a straight-line
    // seed) places its arithmetic after that def; a var defined elsewhere
    // (the loop-header phi of a loop body) places it at position 0. A
    // pack whose schedule position does not strictly follow the
    // materialization keeps its materialized pointer — fail-closed.
    let mut sib_materialize: Vec<(usize, usize, Instruction)> = Vec::new();
    // Per pack: (base, idx value, disp) when the stream decomposes.
    let mut sib_streams: Vec<Option<(Value, Value, i64)>> = vec![None; plan.packs.len()];
    // The seed store's decomposed anchor, when available.
    let mut anchor_out: Option<(Value, Value, i64)> = None;
    {
        // Phase 1 (read-only): per-pack stream decompositions and the
        // distinct (var, mult, var_ty) materialization needs.
        let block_snapshot = &func.blocks[block_idx];
        let mut def_pos_local: FxHashMap<u32, usize> = FxHashMap::default();
        for (i, inst) in block_snapshot.instructions.iter().enumerate() {
            if let Some(d) = inst.dest() {
                def_pos_local.insert(d.0, i);
            }
        }
        let mut need: Vec<(Value, u64, IrType)> = Vec::new();
        // streams_stage: (base, var, mult, disp)
        let mut streams_stage: Vec<Option<(Value, Value, u64, i64)>> = vec![None; plan.packs.len()];
        let mut anchor_stage: Option<(Value, Value, u64, i64)> = None;
        for (pi, p) in plan.packs.iter().enumerate() {
            let PackKind::MemLoad { ptrs, .. } = &p.kind else {
                continue;
            };
            let Some(a) = eval_sym_addr(block_snapshot, &def_pos_local, ptrs[0]) else {
                continue;
            };
            let Some(v) = a.var else { continue };
            if a.mult == 0 {
                continue;
            }
            // The var's TYPE comes from its defining instruction ANYWHERE
            // in the function (the loop IV is a HEADER phi — the body
            // block under SLP only USES it). SSA dominance is automatic
            // for used values; only the INTRA-BLOCK position needs care
            // (handled at materialization below). Values without an
            // integer def in this function (opaque foreign vars) keep
            // the materialized pointer — fail-closed.
            let Some(var_ty) = lookup_var_ty(func, v) else {
                continue;
            };
            need.push((v, a.mult, var_ty));
            streams_stage[pi] = Some((a.base, v, a.mult, a.off));
        }
        // The seed store's anchor stream decomposes the same way (the
        // sha256 schedule's store `m[i]` shares the loads' (i, 4) index).
        if let Some(a) = eval_sym_addr(block_snapshot, &def_pos_local, cand.anchor_ptr) {
            if let Some(v) = a.var {
                if a.mult != 0 {
                    if let Some(var_ty) = lookup_var_ty(func, v) {
                        need.push((v, a.mult, var_ty));
                        anchor_stage = Some((a.base, v, a.mult, a.off));
                    }
                }
            }
        }
        // Phase 2 (minting): one Cast(+Shl/Mul) chain per distinct
        // (var, mult), shared by every pack of the plan. The widening
        // cast is shared per VAR (mult=1 uses it directly).
        let mut sib_index: FxHashMap<(u32, u64), (Value, usize)> = FxHashMap::default();
        let mut cast_of: FxHashMap<u32, (Value, usize)> = FxHashMap::default();
        for (v, mult, var_ty) in need {
            if sib_index.contains_key(&(v.0, mult)) {
                continue;
            }
            // Placement: after the var's def when it is defined in THIS
            // block, else at the top (a dominating def — the loop phi).
            let mat_pos = match def_pos_local.get(&v.0) {
                Some(&d) => d + 1,
                None => 0,
            };
            let (cast_dest, cast_pos) = if var_ty == IrType::I64 {
                (v, mat_pos)
            } else if let Some(&(cv, cp)) = cast_of.get(&v.0) {
                (cv, cp)
            } else {
                let cv = Value(func.next_value_id);
                func.next_value_id += 1;
                sib_materialize.push((
                    mat_pos,
                    sib_materialize.len(),
                    Instruction::Cast {
                        dest: cv,
                        src: Operand::Value(v),
                        from_ty: var_ty,
                        to_ty: IrType::I64,
                    },
                ));
                cast_of.insert(v.0, (cv, mat_pos));
                (cv, mat_pos)
            };
            if mult == 1 {
                sib_index.insert((v.0, mult), (cast_dest, cast_pos));
            } else {
                let idx = Value(func.next_value_id);
                func.next_value_id += 1;
                let op = if mult.is_power_of_two() {
                    Instruction::BinOp {
                        dest: idx,
                        op: IrBinOp::Shl,
                        lhs: Operand::Value(cast_dest),
                        rhs: Operand::Const(IrConst::I64(mult.trailing_zeros() as i64)),
                        ty: IrType::I64,
                    }
                } else {
                    Instruction::BinOp {
                        dest: idx,
                        op: IrBinOp::Mul,
                        lhs: Operand::Value(cast_dest),
                        rhs: Operand::Const(IrConst::I64(mult as i64)),
                        ty: IrType::I64,
                    }
                };
                let idx_pos = cast_pos.max(mat_pos).max(mat_pos);
                sib_materialize.push((idx_pos, sib_materialize.len(), op));
                sib_index.insert((v.0, mult), (idx, idx_pos));
            }
        }
        // Phase 3: resolve each pack's (base, idx, disp) triple — the
        // pack's schedule position must STRICTLY follow the idx's
        // materialization position (same-position ordering is the
        // (position, order) sort; the materializations carry their own
        // increasing orders, so strictly-later positions are the only
        // unambiguous form).
        for (pi, s) in streams_stage.into_iter().enumerate() {
            if let Some((base, v, mult, disp)) = s {
                if let Some(&(idx, idx_pos)) = sib_index.get(&(v.0, mult)) {
                    if plan.packs[pi].sched > idx_pos {
                        sib_streams[pi] = Some((base, idx, disp));
                    }
                }
            }
        }
        if let Some((base, v, mult, disp)) = anchor_stage {
            if let Some(&(idx, idx_pos)) = sib_index.get(&(v.0, mult)) {
                let store_pos = *cand.store_idx.iter().max().unwrap();
                if store_pos > idx_pos {
                    anchor_out = Some((base, idx, disp));
                }
            }
        }
    }

    // Insertion batches: (position, order, instruction). Sorted by
    // (position, order); dependencies land first within a slot.
    let mut inserts: Vec<(usize, usize, Instruction)> = Vec::new();
    inserts.extend(sib_materialize);
    for (pi, p) in plan.packs.iter().enumerate() {
        let dest = vec_dest[pi];
        let inst = match &p.kind {
            // No instruction: the pack's vector IS the forwarded value.
            // (No extracts ride with it either — `lane_vals` is empty.)
            PackKind::Forward { .. } => continue,
            PackKind::Fma { negate, acc, a, b } => Instruction::Intrinsic {
                dest: Some(dest),
                op: match (negate, cand.ty) {
                    (false, IrType::F64) => IntrinsicOp::VecFmaF64x2,
                    (true, IrType::F64) => IntrinsicOp::VecFnmaF64x2,
                    (false, IrType::F32) => IntrinsicOp::VecFmaF32x4,
                    (true, IrType::F32) => IntrinsicOp::VecFnmaF32x4,
                    // The pack builder only creates Fma packs for F64/F32
                    // lanes (the contraction's type gate).
                    _ => unreachable!("Fma pack with non-FP lane type"),
                },
                dest_ptr: None,
                args: vec![
                    Operand::Value(vec_dest[*a]),
                    Operand::Value(vec_dest[*b]),
                    Operand::Value(vec_dest[*acc]),
                ],
            },
            PackKind::MemLoad { ptrs, .. } => {
                // SIB form when the stream decomposed: (base, idx, disp)
                // folds into `disp(%base,%idx)` in the back end; otherwise
                // lane 0's materialized pointer anchors the access. Either
                // way the vector reads exactly [off0, off0 + width*size) —
                // the union of the scalar lane reads, never more.
                let args = match sib_streams.get(pi).and_then(|s| *s) {
                    Some((base, idx, disp)) => vec![
                        Operand::Value(base),
                        Operand::Value(idx),
                        Operand::Const(IrConst::I64(disp)),
                    ],
                    None => vec![Operand::Value(ptrs[0]), Operand::Const(IrConst::I64(0))],
                };
                Instruction::Intrinsic {
                    dest: Some(dest),
                    op: fam.load,
                    dest_ptr: None,
                    args,
                }
            }
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
            PackKind::FpNeg { vec_op, val } => {
                // The Const(-0.0) operand selects the backend's sign-mask
                // memory-operand fast path (one instruction, GCC parity).
                let mz = minus_zero_const(cand.ty)
                    .expect("FpNeg packs are only built for F32/F64 lanes");
                Instruction::Intrinsic {
                    dest: Some(dest),
                    op: *vec_op,
                    dest_ptr: None,
                    args: vec![Operand::Value(vec_dest[*val]), Operand::Const(mz)],
                }
            }
            PackKind::CmpBlendv {
                cmp_op,
                blend_op,
                pred,
                lhs,
                rhs,
                tv,
                fv,
                ..
            } => {
                // TWO intrinsics at the same schedule slot: the compare
                // first, then the select reading its mask (the stable
                // (sched, order) sort preserves this push order; the
                // extracts ride at p.order+1+li, after both).
                let mask = cmp_mask_dest[&pi];
                inserts.push((
                    p.sched,
                    p.order,
                    Instruction::Intrinsic {
                        dest: Some(mask),
                        op: *cmp_op,
                        dest_ptr: None,
                        args: vec![
                            Operand::Value(vec_dest[*lhs]),
                            Operand::Value(vec_dest[*rhs]),
                            Operand::Const(IrConst::I32(*pred as i32)),
                        ],
                    },
                ));
                Instruction::Intrinsic {
                    dest: Some(dest),
                    op: *blend_op,
                    dest_ptr: None,
                    args: vec![
                        Operand::Value(vec_dest[*fv]),
                        Operand::Value(vec_dest[*tv]),
                        Operand::Value(mask),
                    ],
                }
            }
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
    // the 3-arg form and ignores the redundant `dest_ptr`. With a
    // decomposed anchor the args become the SIB 4-arg form
    // [vector, base, index, disp] — `emit_vec_store_addr`'s ≥3-arg path
    // folds it to `disp(%base,%index)`, and the anchor GEP chain dies.
    let store_args = match anchor_out {
        Some((base, idx, disp)) => vec![
            Operand::Value(vec_dest[plan.root]),
            Operand::Value(base),
            Operand::Value(idx),
            Operand::Const(IrConst::I64(disp)),
        ],
        None => vec![
            Operand::Value(vec_dest[plan.root]),
            Operand::Value(cand.anchor_ptr),
            Operand::Const(IrConst::I64(0)),
        ],
    };
    inserts.push((
        *cand.store_idx.iter().max().unwrap(),
        10_000,
        Instruction::Intrinsic {
            dest: None,
            op: fam.store,
            dest_ptr: Some(cand.anchor_ptr),
            args: store_args,
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

    // Cross-block use rewrite (rule (b)'s v6 relaxation): the extracts
    // live in THIS block and dominate every external use (see the proof
    // in the legality walk), so every OTHER block's uses of the replaced
    // lanes — instructions, phi incomings, terminators — are rewritten to
    // the extract values through the canonical mutation walkers. The map
    // is usually tiny; the per-block scan is a cheap hash-miss walk.
    if !extract_dest.is_empty() {
        for (bi, blk) in func.blocks.iter_mut().enumerate() {
            if bi == block_idx {
                continue;
            }
            for inst in blk.instructions.iter_mut() {
                rewrite_uses_in_place(inst, &extract_dest);
            }
            rewrite_term_uses_in_place(&mut blk.terminator, &extract_dest);
        }
    }

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
                PackKind::Forward { val } => format!("Forward(v{})", val.0),
                PackKind::Fma { negate, acc, a, b } => {
                    format!(
                        "Fma({} acc{} a{} b{})",
                        if *negate { "-" } else { "+" },
                        acc,
                        a,
                        b
                    )
                }
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
                PackKind::FpNeg { .. } => "FpNeg".into(),
                PackKind::CmpBlendv { .. } => "CmpBlendv".into(),
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

/// In-place variants of `rewrite_uses`/`rewrite_term_uses` for the
/// cross-block rewrite (rule (b)'s v6 relaxation): the same canonical
/// mutation walkers, applied to OTHER blocks' instructions and
/// terminators without moving them.
fn rewrite_uses_in_place(inst: &mut Instruction, map: &FxHashMap<u32, Value>) {
    if map.is_empty() {
        return;
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
}

fn rewrite_term_uses_in_place(term: &mut Terminator, map: &FxHashMap<u32, Value>) {
    if map.is_empty() {
        return;
    }
    term.for_each_operand_mut(|op| {
        if let Operand::Value(v) = op {
            if let Some(&nv) = map.get(&v.0) {
                *op = Operand::Value(nv);
            }
        }
    });
}
