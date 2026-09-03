//! Backedge partial-redundancy elimination (BEPRE): carry a loop-bottom
//! computation of f(next_value) into the loop-top use of f(phi) through a
//! new header phi.
//!
//! In a recurrence loop like Mandelbrot's
//!   p  = phi(init, v)              // header
//!   e  = fmul p, p                 // top use (iteration difference)
//!   v  = ...                       // next value, computed mid-body
//!   e2 = fmul v, v                 // bottom use (escape/magnitude test)
//!        ... backedge to header
//! e and e2 are the same expression one iteration apart: on iteration n,
//! e == e2 of iteration n-1. Rewriting e's uses to the new phi
//!   q  = phi(f(init) [preheader], e2 [latch])
//! removes e's computation from the loop entirely. GCC often reaches the same
//! shape via loop rotation + CSE.
//!
//! # Soundness argument
//!
//! Let p_n be the phi's value in iteration n (p_0 = init, p_{n+1} = next_n)
//! and q_n the new phi's value. q_0 = f(init) = f(p_0). For n > 0,
//! q_n = e2_{n-1} = f(next_{n-1}, inv) = f(p_n, inv) = e_n. Hence q == e at
//! every program point dominated by e's definition (q is defined in the
//! header, which dominates the whole body), and renaming e -> q is
//! semantics-preserving regardless of where e is used, how often e is
//! executed, or whether e is used by e2 itself (`x = x*x` chains).
//!
//! The argument survives *simultaneous* rewrites, including chains
//! (e2 of one rewrite is e of another) and phi swaps (two rewrites feeding
//! each other): every rename is justified by the invariant of the renamed
//! value at the renamed use, and all invariants are established by a joint
//! induction over execution order. That is why this pass batches all renames
//! into one pass instead of pruning chains (the previous version dropped
//! every chained rewrite, and its per-rewrite renaming could leave a
//! *deleted* value referenced from a later rewrite's preheader computation
//! when an inner loop's init operand was an outer loop's top expression).
//!
//! Requirements checked per candidate:
//! * every non-constant operand of e is either a header phi with exactly the
//!   (preheader, latch) incomings of a single-latch loop, or a loop-invariant
//!   value whose definition dominates the preheader (both operands may be
//!   phis: `zr*zi` with `zr'*zi'` at the bottom is a valid candidate);
//! * e2 has the same op/type with each phi replaced by its latch value, and
//!   e2's block dominates the latch, so it is available on every backedge;
//! * the op cannot trap (f(init) is speculated in the preheader even for
//!   zero-trip loops; FP division is kept out to match GCC's default
//!   -ftrapping-math speculation policy).
//!
//! # Profitability model
//!
//! Fusion-aware and target-aware: the top expression must be an instruction
//! the emitter really materializes (not a multiply absorbed into
//! fmadd/fmsub/madd/msub, and on x86 not a `x*2^k`/`x<<k` absorbed into an
//! LEA/SIB scale by its add/GEP consumer), and the bottom expression must
//! not lose such an absorption by gaining the phi use. Integer PRE is on by
//! default (measured 1.14x on `backedge_pre_int_recurrence.c`); FP PRE fires
//! by default only when the removed expression has >= 2 direct uses, since
//! the singly-used Mandelbrot square measured slower on x86-64 even after the
//! phi coalescer was taught the cross-block shape
//! (`engineering/evidence/levkropp/backedge_pre_x86_spike.md`).
//! `CCC_BEPRE_FP=1` keeps the broad FP research path. A per-loop
//! loop-carried-phi budget per register class stops the pass from adding a
//! carried value to loops that are already at the allocator's limit, where
//! one saved ALU op cannot pay for a spill.
//!
//! The pass iterates to a bounded fixpoint: a rewrite turns `f(p)` into a
//! header phi, which makes `g(f(p))` a fresh candidate against `g(f(v))`.
//!
//! Kill switches: `CCC_DISABLE_PASSES=bepre` or `CCC_NO_BEPRE=1`.
//! Explainability: `CCC_DEBUG_BEPRE=1` prints every applied rewrite and every
//! rejected candidate that had a matching bottom expression, with the reason.

use super::loop_analysis::{self, DominanceChecker, NaturalLoop};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::{target_elf_machine, target_ptr_size, IrType};
use crate::ir::analysis::CfgAnalysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Value,
};

/// Upper bound on fixpoint rounds. Each applied rewrite strictly removes one
/// in-loop non-phi instruction, so termination does not depend on this; it
/// only bounds compile time for pathological inputs.
const MAX_ROUNDS: usize = 3;

/// Instruction budget for the dominator-chain search that reuses an
/// already-available `f(init)` instead of materializing a preheader copy.
const AVAILABLE_EXPR_SCAN_BUDGET: usize = 512;

// ── Environment / target configuration ──────────────────────────────────────

#[derive(Clone, Copy)]
struct EnvConfig {
    disabled: bool,
    /// `CCC_BEPRE_FP=1`: enable FP PRE for singly-used top expressions too.
    fp_broad: bool,
    debug: bool,
}

impl EnvConfig {
    /// Read once per function (not per candidate as before): env access takes
    /// the process env lock, and this is called from the innermost scan loop.
    fn read() -> Self {
        Self {
            disabled: std::env::var_os("CCC_NO_BEPRE").is_some(),
            fp_broad: std::env::var_os("CCC_BEPRE_FP").is_some(),
            debug: std::env::var_os("CCC_DEBUG_BEPRE").is_some(),
        }
    }
}

const EM_386: u16 = 3;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
const EM_RISCV: u16 = 243;

/// What the current backend can absorb into a consumer, and how many
/// loop-carried values per register class a loop can hold before this pass
/// stops adding more. The pass has no `Target` parameter (it is driven by
/// `for_each_function`), so it reads the thread-local ELF machine that the
/// driver sets before any pass runs.
#[derive(Clone, Copy)]
struct TargetModel {
    /// Integer `mul` feeding `add`/`sub` fuses (AArch64 madd/msub).
    int_madd: bool,
    /// FP `mul` feeding `add`/`sub` may contract (fmadd/fmsub/fnmsub).
    fp_fma: bool,
    /// `x << k` / `x * 2^k` with `k <= max_scale_shift` feeding an add or a
    /// GEP offset is absorbed into the consumer (LEA/SIB scale on x86,
    /// shifted-register operands on AArch64, Zba `shNadd` on RISC-V).
    max_scale_shift: u32,
    /// `a - (x << k)` is absorbed too (AArch64 `sub ..., lsl #k`); x86's LEA
    /// cannot subtract.
    scaled_sub: bool,
    /// Loop-carried phi budget (weighted: 128-bit integers count twice).
    int_phi_budget: usize,
    fp_phi_budget: usize,
}

impl TargetModel {
    fn detect() -> Self {
        match target_elf_machine() {
            // 15 allocatable GPRs minus frame pointer; 16 XMM.
            EM_X86_64 => Self {
                int_madd: false,
                fp_fma: true,
                max_scale_shift: 3,
                scaled_sub: false,
                int_phi_budget: 10,
                fp_phi_budget: 11,
            },
            // 6-7 GPRs, 8 XMM, no FMA.
            EM_386 => Self {
                int_madd: false,
                fp_fma: false,
                max_scale_shift: 3,
                scaled_sub: false,
                int_phi_budget: 4,
                fp_phi_budget: 5,
            },
            // 28 allocatable GPRs, 32 FP/SIMD.
            EM_AARCH64 => Self {
                int_madd: true,
                fp_fma: true,
                max_scale_shift: 63,
                scaled_sub: true,
                int_phi_budget: 20,
                fp_phi_budget: 22,
            },
            EM_RISCV => Self {
                int_madd: false,
                fp_fma: true,
                max_scale_shift: 3,
                scaled_sub: false,
                int_phi_budget: 20,
                fp_phi_budget: 22,
            },
            // Unknown target: assume every fusion exists (never breaks one)
            // and x86-64-like register budgets.
            _ => Self {
                int_madd: true,
                fp_fma: true,
                max_scale_shift: 3,
                scaled_sub: true,
                int_phi_budget: 10,
                fp_phi_budget: 11,
            },
        }
    }
}

// ── Expression keys ─────────────────────────────────────────────────────────

/// Operand identity for expression matching. Constants are normalized to
/// the operation's type so `I32(5)` and `I64(5)` (or `Zero` and `F64(0.0)`)
/// in an op of the same type compare equal.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, PartialOrd, Ord)]
enum OperandKey {
    Val(u32),
    Con(i128),
}

/// Expression kinds this pass carries: non-trapping BinOps and value casts
/// (integer promotions sit between a byte-typed phi and its arithmetic, so
/// without casts no byte-loop recurrence is ever a candidate).
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
enum ExprOp {
    Bin(IrBinOp),
    /// `Cast { from_ty }`; the result type lives in `ExprKey::ty`.
    Cast(IrType),
}

impl ExprOp {
    fn is_commutative(self) -> bool {
        match self {
            ExprOp::Bin(b) => b.is_commutative(),
            ExprOp::Cast(_) => false,
        }
    }

    /// Type in which the first operand's constants are normalized.
    fn lhs_ty(self, result_ty: IrType) -> IrType {
        match self {
            ExprOp::Bin(_) => result_ty,
            ExprOp::Cast(from) => from,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
struct ExprKey {
    op: ExprOp,
    ty: IrType,
    lhs: OperandKey,
    rhs: OperandKey,
}

impl ExprKey {
    fn new(op: ExprOp, ty: IrType, mut lhs: OperandKey, mut rhs: OperandKey) -> Self {
        if op.is_commutative() && rhs < lhs {
            std::mem::swap(&mut lhs, &mut rhs);
        }
        Self { op, ty, lhs, rhs }
    }
}

/// The uniform view of a candidate instruction (top, bottom, or an
/// already-available preheader computation).
#[derive(Clone, Copy)]
struct Parts {
    dest: Value,
    op: ExprOp,
    ty: IrType,
    lhs: Operand,
    /// `None` for casts.
    rhs: Option<Operand>,
}

impl Parts {
    fn of(inst: &Instruction) -> Option<Parts> {
        match inst {
            Instruction::BinOp { dest, op, lhs, rhs, ty } => {
                if op.can_trap() || !pre_type_ok(*ty) {
                    return None;
                }
                Some(Parts { dest: *dest, op: ExprOp::Bin(*op), ty: *ty, lhs: *lhs, rhs: Some(*rhs) })
            }
            Instruction::Cast { dest, src, from_ty, to_ty } => {
                if !cast_ok(*from_ty, *to_ty) {
                    return None;
                }
                Some(Parts { dest: *dest, op: ExprOp::Cast(*from_ty), ty: *to_ty, lhs: *src, rhs: None })
            }
            _ => None,
        }
    }

    fn key_of(op: ExprOp, ty: IrType, lhs: &Operand, rhs: Option<&Operand>) -> Option<ExprKey> {
        let kl = operand_key(lhs, op.lhs_ty(ty))?;
        let kr = match rhs {
            Some(r) => operand_key(r, ty)?,
            None => OperandKey::Con(0),
        };
        Some(ExprKey::new(op, ty, kl, kr))
    }

    fn key(&self) -> Option<ExprKey> {
        Self::key_of(self.op, self.ty, &self.lhs, self.rhs.as_ref())
    }

    fn build(op: ExprOp, ty: IrType, dest: Value, lhs: Operand, rhs: Option<Operand>) -> Instruction {
        match (op, rhs) {
            (ExprOp::Bin(b), Some(rhs)) => Instruction::BinOp { dest, op: b, lhs, rhs, ty },
            (ExprOp::Cast(from), _) => Instruction::Cast { dest, src: lhs, from_ty: from, to_ty: ty },
            // A binary op without a right operand cannot come out of `of`.
            (ExprOp::Bin(b), None) => Instruction::BinOp { dest, op: b, lhs, rhs: lhs, ty },
        }
    }
}

/// Casts carried across the backedge: value conversions between the carried
/// types (never trapping on any supported target: x86 `cvtt*` returns the
/// integer indefinite, AArch64/RISC-V saturate).
fn cast_ok(from: IrType, to: IrType) -> bool {
    from != to && pre_type_ok(from) && pre_type_ok(to)
}

/// Integer truncations and same-size integer reinterpretations cost no
/// instruction (the backend reads the low bits), so removing one from the
/// loop saves nothing and a carried phi would be pure cost.
fn cast_is_free(from: IrType, to: IrType) -> bool {
    !from.is_float() && !to.is_float() && to.size() <= from.size()
}

/// Exact folding of an integer-to-integer cast: normalize the source by its
/// own signedness and width, then store through the canonical constructor.
fn fold_const_cast(c: &IrConst, from: IrType, to: IrType) -> Option<IrConst> {
    let (fb, tb) = (int_bits(from)?, int_bits(to)?);
    let v = c.to_i128()?;
    let v = if from.is_unsigned() || from == IrType::Ptr { zext_to(v, fb) } else { sext_to(v, fb) };
    Some(if tb == 128 { IrConst::I128(v) } else { IrConst::from_i64(v as i64, to) })
}

/// Types this pass carries across the backedge. Decimal and x87 long-double
/// values are excluded: their BinOps lower to libcalls or x87 stack code
/// where an extra phi is not a register-to-register copy.
fn pre_type_ok(ty: IrType) -> bool {
    matches!(
        ty,
        IrType::I8
            | IrType::I16
            | IrType::I32
            | IrType::I64
            | IrType::I128
            | IrType::U8
            | IrType::U16
            | IrType::U32
            | IrType::U64
            | IrType::U128
            | IrType::Ptr
            | IrType::F32
            | IrType::F64
    )
}

/// Bit width of an integer type (pointer width follows the target).
fn int_bits(ty: IrType) -> Option<u32> {
    Some(match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        IrType::I32 | IrType::U32 => 32,
        IrType::I64 | IrType::U64 => 64,
        IrType::I128 | IrType::U128 => 128,
        IrType::Ptr => (target_ptr_size() * 8) as u32,
        _ => return None,
    })
}

#[inline]
fn sext_to(v: i128, bits: u32) -> i128 {
    if bits >= 128 {
        v
    } else {
        let s = 128 - bits;
        (v << s) >> s
    }
}

#[inline]
fn zext_to(v: i128, bits: u32) -> i128 {
    if bits >= 128 {
        v
    } else {
        v & ((1i128 << bits) - 1)
    }
}

fn const_key(c: &IrConst, ty: IrType) -> Option<OperandKey> {
    if let Some(bits) = int_bits(ty) {
        // Sign-normalize to the operation width: the same low bits mean the
        // same operand regardless of how the constant was stored.
        return c.to_i128().map(|v| OperandKey::Con(sext_to(v, bits)));
    }
    match (ty, c) {
        (IrType::F32, IrConst::F32(f)) => Some(OperandKey::Con(f.to_bits() as i128)),
        (IrType::F64, IrConst::F64(f)) => Some(OperandKey::Con(f.to_bits() as i128)),
        (IrType::F32 | IrType::F64, IrConst::Zero) => Some(OperandKey::Con(0)),
        // LongDouble carries a lossy f64 plus exact bytes; decimals never
        // reach binary BinOps. Skip rather than risk equating distinct values.
        _ => None,
    }
}

fn operand_key(op: &Operand, ty: IrType) -> Option<OperandKey> {
    match op {
        Operand::Value(v) => Some(OperandKey::Val(v.0)),
        Operand::Const(c) => const_key(c, ty),
    }
}

/// Register class and weight of a carried value for the phi budget.
fn class_weight(ty: IrType) -> (bool, usize) {
    match ty {
        IrType::I128 | IrType::U128 => (false, 2),
        t if t.is_float() => (true, 1),
        _ => (false, 1),
    }
}

// ── Constant folding of the preheader expression ────────────────────────────

fn fp_operand(c: &IrConst, ty: IrType) -> Option<f64> {
    match (ty, c) {
        (IrType::F64, IrConst::F64(f)) => Some(*f),
        (IrType::F32, IrConst::F32(f)) => Some(*f as f64),
        (IrType::F32 | IrType::F64, IrConst::Zero) => Some(0.0),
        _ => None,
    }
}

/// Fold `f(init)` when both operands are constants, so the header phi gets a
/// constant incoming instead of a preheader temp. This matters for
/// recurrences that start from zero (Mandelbrot): the naive `0.0 * 0.0`
/// lengthened the FP live set before the first iteration enough to spill an
/// outer-loop constant on x86-64.
///
/// Integer results go through `IrConst::from_i64(_, ty)`, the canonical
/// constructor that stores unsigned values zero-extended (the previous
/// `IrConst::I8(v as i8)` for `U8` produced `I8(-1)` for 255, which
/// `to_i64()` later reads back as -1). Shifts are folded only for in-range
/// amounts, with the left operand normalized to the operation width first
/// (a `U32` stored as `I32(-1)` must shift as `0xFFFF_FFFF`, not as -1i64).
/// FP folding is limited to finite operands and to Add/Sub/Mul, whose
/// host evaluation is IEEE-identical to the target's default rounding.
fn fold_const_binop(op: IrBinOp, lhs: &Operand, rhs: &Operand, ty: IrType) -> Option<IrConst> {
    let (Operand::Const(a), Operand::Const(b)) = (lhs, rhs) else {
        return None;
    };
    match ty {
        IrType::F64 | IrType::F32 => {
            let (x, y) = (fp_operand(a, ty)?, fp_operand(b, ty)?);
            if !(x.is_finite() && y.is_finite()) {
                return None;
            }
            if ty == IrType::F32 {
                let (x, y) = (x as f32, y as f32);
                let r = match op {
                    IrBinOp::Add => x + y,
                    IrBinOp::Sub => x - y,
                    IrBinOp::Mul => x * y,
                    _ => return None,
                };
                Some(IrConst::F32(r))
            } else {
                let r = match op {
                    IrBinOp::Add => x + y,
                    IrBinOp::Sub => x - y,
                    IrBinOp::Mul => x * y,
                    _ => return None,
                };
                Some(IrConst::F64(r))
            }
        }
        _ => {
            let bits = int_bits(ty)?;
            let (x, y) = (a.to_i128()?, b.to_i128()?);
            let r = match op {
                IrBinOp::Add
                | IrBinOp::Sub
                | IrBinOp::Mul
                | IrBinOp::And
                | IrBinOp::Or
                | IrBinOp::Xor => op.eval_i128(x, y)?,
                IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr => {
                    if y < 0 || y >= i128::from(bits) {
                        return None;
                    }
                    let xn = if op == IrBinOp::LShr { zext_to(x, bits) } else { sext_to(x, bits) };
                    op.eval_i128(xn, y)?
                }
                _ => return None,
            };
            Some(if bits == 128 { IrConst::I128(r) } else { IrConst::from_i64(r as i64, ty) })
        }
    }
}

// ── Definition / use tables ─────────────────────────────────────────────────

/// Flat per-value tables (indexed by value id, sized from
/// `sound_next_value_id`) replacing three hash maps on the hot scan path.
struct Defs {
    /// (block, index) of the unique definition, or `NONE`.
    site: Vec<(u32, u32)>,
    /// Multiply-defined (malformed IR) or defined by a side channel
    /// (inline-asm output) — never a candidate, never an invariant.
    tainted: Vec<bool>,
    uses: Vec<u32>,
}

const NONE: u32 = u32::MAX;

impl Defs {
    fn build(func: &IrFunction, bound: usize) -> Self {
        let mut d = Defs {
            site: vec![(NONE, NONE); bound],
            tainted: vec![false; bound],
            uses: vec![0; bound],
        };
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                if let Some(v) = inst.dest() {
                    let id = v.0 as usize;
                    if id >= bound || d.site[id].0 != NONE {
                        if id < bound {
                            d.tainted[id] = true;
                        }
                    } else {
                        d.site[id] = (bi as u32, ii as u32);
                    }
                }
                if let Instruction::InlineAsm { outputs, .. } = inst {
                    for (_, v, _) in outputs {
                        if (v.0 as usize) < bound {
                            d.tainted[v.0 as usize] = true;
                        }
                    }
                }
                inst.for_each_used_value(|u| {
                    if (u as usize) < bound {
                        d.uses[u as usize] += 1;
                    }
                });
            }
            block.terminator.for_each_used_value(|u| {
                if (u as usize) < bound {
                    d.uses[u as usize] += 1;
                }
            });
        }
        d
    }

    #[inline]
    fn site(&self, v: u32) -> Option<(usize, usize)> {
        match self.site.get(v as usize) {
            Some(&(b, i)) if b != NONE => Some((b as usize, i as usize)),
            _ => None,
        }
    }

    #[inline]
    fn tainted(&self, v: u32) -> bool {
        self.tainted.get(v as usize).copied().unwrap_or(true)
    }

    #[inline]
    fn uses(&self, v: u32) -> u32 {
        self.uses.get(v as usize).copied().unwrap_or(0)
    }
}

// ── Fusion / absorption model ───────────────────────────────────────────────

fn pow2_shift(c: &IrConst) -> Option<u32> {
    let v = c.to_i128()?;
    if v > 0 && (v & (v - 1)) == 0 {
        Some(v.trailing_zeros())
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Producer {
    /// A real multiply that a consumer add/sub may contract.
    Mul,
    /// `x * 2^k` or `x << k`: an addressing-mode scale for add/GEP consumers.
    Scale,
}

fn operand_is(o: &Operand, v: u32) -> bool {
    matches!(o, Operand::Value(x) if x.0 == v)
}

/// Would the value defined at its (single) definition be absorbed into its
/// single consumer by the backend, so that it never costs an instruction of
/// its own? Mirrors the emitter's adjacency rules: the consumer must follow
/// within a short gap containing only loads/GEPs. Conservative both ways — a
/// wrong answer costs performance, never correctness.
fn absorbed_by_consumer(func: &IrFunction, defs: &Defs, value: u32, tm: &TargetModel) -> bool {
    let Some((b, i)) = defs.site(value) else {
        return false;
    };
    let insts = &func.blocks[b].instructions;
    if let Instruction::Cast { from_ty, to_ty, .. } = &insts[i] {
        // A free cast costs nothing regardless of its use count.
        return cast_is_free(*from_ty, *to_ty);
    }
    if defs.uses(value) != 1 {
        return false;
    }
    let Instruction::BinOp { op, ty, lhs, rhs, .. } = &insts[i] else {
        return false;
    };
    if matches!(ty, IrType::F128 | IrType::I128 | IrType::U128 | IrType::D32 | IrType::D64) {
        return false;
    }
    let is_float = ty.is_float();
    let producer = match op {
        IrBinOp::Mul => {
            let scale = !is_float
                && [lhs, rhs].into_iter().any(|o| {
                    matches!(o, Operand::Const(c)
                        if pow2_shift(c).is_some_and(|k| k <= tm.max_scale_shift))
                });
            let contractible = if is_float { tm.fp_fma } else { tm.int_madd };
            if scale {
                Producer::Scale
            } else if contractible {
                Producer::Mul
            } else {
                return false;
            }
        }
        IrBinOp::Shl if !is_float => match rhs {
            Operand::Const(c)
                if c.to_i128().is_some_and(|k| k >= 0 && k <= i128::from(tm.max_scale_shift)) =>
            {
                Producer::Scale
            }
            _ => return false,
        },
        _ => return false,
    };
    for inst in insts.iter().take(usize::min(i + 4, insts.len())).skip(i + 1) {
        match inst {
            Instruction::GetElementPtr { offset, .. } => {
                if operand_is(offset, value) {
                    return producer == Producer::Scale;
                }
            }
            Instruction::Load { .. } => {}
            Instruction::BinOp { op: cop, lhs: cl, rhs: cr, ty: cty, .. }
                if matches!(cop, IrBinOp::Add | IrBinOp::Sub) && cty == ty =>
            {
                let lhs_is = operand_is(cl, value);
                let rhs_is = operand_is(cr, value);
                if !lhs_is && !rhs_is {
                    return false;
                }
                return match (producer, cop) {
                    (Producer::Mul, IrBinOp::Add) => true,
                    // a - b*c fuses as msub/fmsub; b*c - a only as fnmsub (FP).
                    (Producer::Mul, _) => rhs_is || is_float,
                    (Producer::Scale, IrBinOp::Add) => true,
                    (Producer::Scale, _) => rhs_is && tm.scaled_sub,
                };
            }
            _ => return false,
        }
    }
    false
}

// ── Block surgery with source-span maintenance ──────────────────────────────

/// `source_spans` is parallel to `instructions` when debug info is tracked.
/// Keep it parallel through every insert/remove instead of dropping a whole
/// block's line table (the previous behaviour) — the convention used by
/// `restore_phi_prefix`: only touch spans when the lengths agree.
#[inline]
fn spans_synced(b: &BasicBlock) -> bool {
    !b.source_spans.is_empty() && b.source_spans.len() == b.instructions.len()
}

fn insert_inst(b: &mut BasicBlock, idx: usize, inst: Instruction, span: Option<Span>) {
    if spans_synced(b) {
        let s = span
            .or_else(|| b.source_spans.get(idx.saturating_sub(1)).copied())
            .or_else(|| b.source_spans.get(idx).copied())
            .unwrap_or_else(Span::dummy);
        b.source_spans.insert(idx, s);
    } else if !b.source_spans.is_empty() {
        b.source_spans.clear();
    }
    b.instructions.insert(idx, inst);
}

fn remove_inst(b: &mut BasicBlock, idx: usize) -> (Instruction, Option<Span>) {
    let span = if spans_synced(b) {
        Some(b.source_spans.remove(idx))
    } else {
        if !b.source_spans.is_empty() {
            b.source_spans.clear();
        }
        None
    };
    (b.instructions.remove(idx), span)
}

fn span_of_def(b: &BasicBlock, v: Value) -> Option<Span> {
    if !spans_synced(b) {
        return None;
    }
    b.instructions
        .iter()
        .position(|i| i.dest() == Some(v))
        .and_then(|idx| b.source_spans.get(idx).copied())
}

fn first_non_phi(b: &BasicBlock) -> usize {
    b.instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
        .unwrap_or(b.instructions.len())
}

/// Index of the candidate-shaped (BinOp/Cast) instruction defining `v`.
fn expr_index(b: &BasicBlock, v: Value) -> Option<usize> {
    b.instructions.iter().position(|i| {
        matches!(i, Instruction::BinOp { dest, .. } | Instruction::Cast { dest, .. } if *dest == v)
    })
}

// ── Planning ────────────────────────────────────────────────────────────────

/// A planned rewrite: replace uses of `e_dest` with a new header phi whose
/// latch incoming is `e2_dest` and whose preheader incoming is `f(init)`.
struct Rewrite {
    header: usize,
    preheader: usize,
    preheader_label: BlockId,
    latch_label: BlockId,
    e_block: usize,
    e_dest: Value,
    e2_block: usize,
    e2_dest: Value,
    op: ExprOp,
    ty: IrType,
    /// Operands of the preheader computation (each phi replaced by its init).
    init_lhs: Operand,
    init_rhs: Option<Operand>,
}

#[derive(Clone, Copy)]
struct PhiInfo {
    init: Operand,
    next: Value,
}

struct Ctx<'a> {
    func: &'a IrFunction,
    cfg: &'a CfgAnalysis,
    dom: &'a DominanceChecker,
    defs: &'a Defs,
    tm: &'a TargetModel,
    env: &'a EnvConfig,
}

fn plan_loop(cx: &Ctx<'_>, lp: &NaturalLoop, out: &mut Vec<Rewrite>) {
    let func = cx.func;
    let header = lp.header;
    let Some(preheader) = loop_analysis::find_preheader(header, &lp.body, &cx.cfg.preds) else {
        return;
    };
    let Some(latch) = lp.single_latch(&cx.cfg.preds) else {
        return;
    };
    let preheader_label = func.blocks[preheader].label;
    let latch_label = func.blocks[latch].label;

    // A value usable by the preheader computation: a constant, or a value
    // defined outside the loop whose definition dominates the preheader.
    let available = |o: &Operand| -> bool {
        match o {
            Operand::Const(_) => true,
            Operand::Value(v) => match cx.defs.site(v.0) {
                Some((db, _)) => {
                    !cx.defs.tainted(v.0)
                        && !lp.body.contains(&db)
                        && cx.dom.dominates(db, preheader)
                }
                None => false,
            },
        }
    };

    // Header phis of the canonical shape, plus the loop-carried budget.
    let mut phis: FxHashMap<u32, PhiInfo> = FxHashMap::default();
    let (mut int_carried, mut fp_carried) = (0usize, 0usize);
    for inst in &func.blocks[header].instructions {
        let Instruction::Phi { dest, ty, incoming } = inst else {
            continue;
        };
        let (fp, w) = class_weight(*ty);
        if fp {
            fp_carried += w;
        } else {
            int_carried += w;
        }
        if incoming.len() != 2 {
            continue;
        }
        let (init, next) = match (incoming[0], incoming[1]) {
            ((a, la), (b, lb)) if la == preheader_label && lb == latch_label => (a, b),
            ((a, la), (b, lb)) if lb == preheader_label && la == latch_label => (b, a),
            _ => continue,
        };
        let Operand::Value(next) = next else {
            continue;
        };
        if next == *dest || cx.defs.tainted(dest.0) || cx.defs.tainted(next.0) || !available(&init) {
            continue;
        }
        phis.insert(dest.0, PhiInfo { init, next });
    }
    if phis.is_empty() {
        return;
    }

    let mut body: Vec<usize> = lp.body.iter().copied().collect();
    body.sort_unstable();

    // Bottom expressions: every eligible BinOp in a block dominating the
    // latch, keyed by (op, ty, operands). First in program order wins.
    let mut bottom: FxHashMap<ExprKey, (usize, Value)> = FxHashMap::default();
    for &bb in &body {
        if !cx.dom.dominates(bb, latch) {
            continue;
        }
        for inst in &func.blocks[bb].instructions {
            let Some(p) = Parts::of(inst) else { continue };
            if cx.defs.tainted(p.dest.0) {
                continue;
            }
            if let Some(k) = p.key() {
                bottom.entry(k).or_insert((bb, p.dest));
            }
        }
    }
    if bottom.is_empty() {
        return;
    }

    // Operand classification: (bottom operand, preheader operand, is_phi).
    let classify = |o: &Operand| -> Option<(Operand, Operand, bool)> {
        match o {
            Operand::Const(_) => Some((*o, *o, false)),
            Operand::Value(v) => {
                if let Some(pi) = phis.get(&v.0) {
                    Some((Operand::Value(pi.next), pi.init, true))
                } else if available(o) {
                    Some((*o, *o, false))
                } else {
                    None
                }
            }
        }
    };

    let explain = |e: Value, why: &str| {
        if cx.env.debug {
            eprintln!("[BEPRE] func={} skip v{}: {}", func.name, e.0, why);
        }
    };

    for &eb in &body {
        for inst in &func.blocks[eb].instructions {
            let Some(p) = Parts::of(inst) else { continue };
            let (e, op, ty) = (p.dest, p.op, p.ty);
            if cx.defs.tainted(e.0) {
                continue;
            }
            let Some((bl, il, pl)) = classify(&p.lhs) else { continue };
            let (br, ir, pr) = match p.rhs.as_ref() {
                Some(r) => match classify(r) {
                    Some((b, i, is_phi)) => (Some(b), Some(i), is_phi),
                    None => continue,
                },
                None => (None, None, false),
            };
            if !pl && !pr {
                continue;
            }
            let Some(bottom_key) = Parts::key_of(op, ty, &bl, br.as_ref()) else {
                continue;
            };
            let Some(&(e2_block, e2)) = bottom.get(&bottom_key) else {
                continue;
            };
            if e2 == e {
                continue;
            }

            // ── Profitability ──
            if ty.is_float() && !cx.env.fp_broad && cx.defs.uses(e.0) < 2 {
                explain(e, "singly-used FP expression (set CCC_BEPRE_FP=1)");
                continue;
            }
            if absorbed_by_consumer(func, cx.defs, e.0, cx.tm) {
                explain(e, "top expression is absorbed by its consumer (no instruction saved)");
                continue;
            }
            if absorbed_by_consumer(func, cx.defs, e2.0, cx.tm) {
                explain(e, "bottom expression would lose its fusion by gaining the phi use");
                continue;
            }
            let (fp, w) = class_weight(ty);
            let (carried, budget) = if fp {
                (&mut fp_carried, cx.tm.fp_phi_budget)
            } else {
                (&mut int_carried, cx.tm.int_phi_budget)
            };
            if *carried + w > budget {
                explain(e, "loop-carried register budget exhausted");
                continue;
            }
            *carried += w;

            out.push(Rewrite {
                header,
                preheader,
                preheader_label,
                latch_label,
                e_block: eb,
                e_dest: e,
                e2_block,
                e2_dest: e2,
                op,
                ty,
                init_lhs: il,
                init_rhs: ir,
            });
        }
    }
}

// ── Application ─────────────────────────────────────────────────────────────

/// Reuse an already-available computation of the preheader expression: walk
/// the dominator chain from the preheader upwards (nearest definitions
/// first) looking for an identical BinOp. GVN cannot merge `x*x` before the
/// loop with `p*p` inside it, so this is a common hit for
/// `y = x*x; for (p = x; ...) { ... p*p ... }`.
fn find_available_expr(
    func: &IrFunction,
    idom: &[usize],
    preheader: usize,
    key: &ExprKey,
    defs: &Defs,
) -> Option<Value> {
    let mut b = preheader;
    let mut budget = AVAILABLE_EXPR_SCAN_BUDGET;
    loop {
        for inst in func.blocks[b].instructions.iter().rev() {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            // Parts::of sees both shapes the key can name (ExprOp::Bin and
            // ExprOp::Cast); a BinOp-only matcher would never reuse an
            // available cast computation and vice versa.
            if let Some(p) = Parts::of(inst) {
                if p.op == key.op
                    && p.ty == key.ty
                    && !defs.tainted(p.dest.0)
                    && p.key().as_ref() == Some(key)
                {
                    return Some(p.dest);
                }
            }
        }
        let p = idom.get(b).copied().unwrap_or(usize::MAX);
        if p == b || p == usize::MAX {
            return None;
        }
        b = p;
    }
}

fn inst_is_safe_to_cross(inst: &Instruction) -> bool {
    match inst {
        Instruction::BinOp { op, .. } => !op.can_trap(),
        Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::Cast { .. }
        | Instruction::Copy { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::Select { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::LabelAddr { .. } => true,
        _ => false,
    }
}

/// If placing an instruction at `at` would split a (mul, add/sub|GEP)
/// pair the emitter fuses by adjacency, return the consumer's index so the
/// caller can move below it.
fn fusion_partner_after(insts: &[Instruction], at: usize, limit: usize) -> Option<usize> {
    if at == 0 {
        return None;
    }
    let Instruction::BinOp { dest, op: IrBinOp::Mul | IrBinOp::Shl, ty, .. } = &insts[at - 1] else {
        return None;
    };
    (at..usize::min(at + 3, limit)).find(|&j| match &insts[j] {
        Instruction::BinOp { op: IrBinOp::Add | IrBinOp::Sub, lhs, rhs, ty: cty, .. } => {
            cty == ty && (operand_is(lhs, dest.0) || operand_is(rhs, dest.0))
        }
        Instruction::GetElementPtr { offset, .. } => operand_is(offset, dest.0),
        _ => false,
    })
}

/// Pull the bottom expression up to its earliest safe point after its
/// operands are available — but never above the last in-block use of the
/// new phi `q`. Keeping `q`'s live range closed before `e2` opens is what
/// lets the phi coalescer bind `q` and `e2` to one register, so the backedge
/// carries no copy at all; the previous scheduler could hoist `e2` above
/// `q`'s uses and manufacture exactly the latch copy the transform was
/// trying to avoid. Also never splits an adjacent fusion pair and never
/// crosses a call (extending `e2` across a call turns it into a callee-saved
/// or spilled value).
fn hoist_bottom_expr(block: &mut BasicBlock, e2: Value, q: Value) -> bool {
    let insts = &block.instructions;
    let Some(cur) = expr_index(block, e2) else {
        return false;
    };
    let Some(p) = Parts::of(&insts[cur]) else {
        return false;
    };
    // Resolve operand definitions against the *current* instruction list:
    // header phis inserted by this pass shift every later index, so any
    // pre-rewrite snapshot would be stale (that stale snapshot once moved
    // `v*v` above `v = x+3` in a rotated self-loop).
    let mut floor = first_non_phi(block);
    for o in std::iter::once(&p.lhs).chain(p.rhs.iter()) {
        if let Operand::Value(v) = o {
            if let Some(di) = insts[..cur].iter().position(|i| i.dest() == Some(*v)) {
                floor = floor.max(di + 1);
            }
        }
    }
    for (j, inst) in insts[..cur].iter().enumerate().skip(floor) {
        let mut uses_q = false;
        inst.for_each_used_value(|u| uses_q |= u == q.0);
        if uses_q {
            floor = j + 1;
        }
    }
    while floor < cur {
        match fusion_partner_after(insts, floor, cur) {
            Some(j) => floor = j + 1,
            None => break,
        }
    }
    if floor >= cur || !insts[floor..cur].iter().all(inst_is_safe_to_cross) {
        return false;
    }
    let (inst, span) = remove_inst(block, cur);
    insert_inst(block, floor, inst, span);
    true
}

fn alloc_value(func: &mut IrFunction) -> Value {
    let v = Value(func.next_value_id);
    func.next_value_id += 1;
    v
}

fn apply(func: &mut IrFunction, rewrites: &[Rewrite], idom: &[usize], defs: &Defs, env: &EnvConfig) -> usize {
    let mut rename: FxHashMap<u32, Value> = FxHashMap::default();
    let mut carried: Vec<Value> = Vec::with_capacity(rewrites.len());
    // Two top expressions in non-dominating blocks (so GVN could not merge
    // them) that share the same bottom expression and the same init operands
    // share one carried phi instead of two identical ones.
    let mut shared: FxHashMap<(usize, u32, ExprKey), Value> = FxHashMap::default();

    // Phase 1: materialize f(init) and the header phi for every rewrite.
    // Nothing is renamed or removed yet, so every planned operand is still
    // valid IR; cross-rewrite references are fixed up by the joint rename.
    for rw in rewrites {
        let e_span = span_of_def(&func.blocks[rw.e_block], rw.e_dest);
        let init_key = Parts::key_of(rw.op, rw.ty, &rw.init_lhs, rw.init_rhs.as_ref());
        let share_key = init_key.map(|k| (rw.header, rw.e2_dest.0, k));
        if let Some(&q) = share_key.as_ref().and_then(|k| shared.get(k)) {
            rename.insert(rw.e_dest.0, q);
            func.fp_expr_tags.remove(&rw.e_dest.0);
            carried.push(q);
            if env.debug {
                eprintln!("[BEPRE] func={} v{} -> shared v{}", func.name, rw.e_dest.0, q.0);
            }
            continue;
        }
        // Fold f(init) where both operands are constants: the per-shape
        // folders (fold_const_binop for ExprOp::Bin, fold_const_cast for
        // ExprOp::Cast — the latter keyed on the SOURCE type the ExprOp
        // carries).
        let folded = match rw.op {
            ExprOp::Bin(b) => rw
                .init_rhs
                .as_ref()
                .and_then(|rhs| fold_const_binop(b, &rw.init_lhs, rhs, rw.ty)),
            ExprOp::Cast(from) => match &rw.init_lhs {
                Operand::Const(c) => fold_const_cast(c, from, rw.ty),
                Operand::Value(_) => None,
            },
        };
        let preheader_incoming = if let Some(c) = folded {
            Operand::Const(c)
        } else if let Some(v) = init_key
            .as_ref()
            .and_then(|k| find_available_expr(func, idom, rw.preheader, k, defs))
        {
            Operand::Value(v)
        } else {
            let pv = alloc_value(func);
            let preh = &mut func.blocks[rw.preheader];
            let at = preh.instructions.len();
            insert_inst(
                preh,
                at,
                Parts::build(rw.op, rw.ty, pv, rw.init_lhs, rw.init_rhs),
                e_span,
            );
            Operand::Value(pv)
        };

        let q = alloc_value(func);
        let hdr = &mut func.blocks[rw.header];
        let pos = first_non_phi(hdr);
        insert_inst(
            hdr,
            pos,
            Instruction::Phi {
                dest: q,
                ty: rw.ty,
                incoming: vec![
                    (preheader_incoming, rw.preheader_label),
                    (Operand::Value(rw.e2_dest), rw.latch_label),
                ],
            },
            e_span,
        );
        rename.insert(rw.e_dest.0, q);
        if let Some(k) = share_key {
            shared.insert(k, q);
        }
        // The removed value's statement tag must not linger: ids are never
        // reused, but a stale FP-contraction tag is still a stale fact.
        func.fp_expr_tags.remove(&rw.e_dest.0);
        carried.push(q);
        if env.debug {
            eprintln!(
                "[BEPRE] func={} v{} -> v{} = phi({:?} [{}], v{} [{}]) in header block {}",
                func.name,
                rw.e_dest.0,
                q.0,
                preheader_incoming,
                rw.preheader_label,
                rw.e2_dest.0,
                rw.latch_label,
                rw.header
            );
        }
    }

    // Phase 2: one joint rename over every operand position, through the
    // canonical visitors (operands, direct Value fields, terminators). The
    // previous hand-written matcher silently skipped Memcpy, atomics,
    // intrinsics, inline-asm inputs, DynAlloca and IndirectBranch, leaving
    // uses of a deleted definition behind.
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            inst.for_each_operand_mut(|o| {
                if let Operand::Value(v) = o {
                    if let Some(&n) = rename.get(&v.0) {
                        *v = n;
                    }
                }
            });
            inst.for_each_value_use_mut(|v| {
                if let Some(&n) = rename.get(&v.0) {
                    *v = n;
                }
            });
        }
        block.terminator.for_each_operand_mut(|o| {
            if let Operand::Value(v) = o {
                if let Some(&n) = rename.get(&v.0) {
                    *v = n;
                }
            }
        });
    }

    // Phase 3: remove the top expressions. Defensive: a definition that is
    // somehow still referenced is kept (redundant but correct) rather than
    // deleted from under its use.
    let mut leaked: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            inst.for_each_used_value(|u| {
                if rename.contains_key(&u) {
                    leaked.insert(u);
                }
            });
        }
        block.terminator.for_each_used_value(|u| {
            if rename.contains_key(&u) {
                leaked.insert(u);
            }
        });
    }
    let mut applied = 0;
    for rw in rewrites {
        if leaked.contains(&rw.e_dest.0) {
            if env.debug {
                eprintln!("[BEPRE] func={} v{} kept: uses remain after rename", func.name, rw.e_dest.0);
            }
            continue;
        }
        let blk = &mut func.blocks[rw.e_block];
        if let Some(idx) = expr_index(blk, rw.e_dest) {
            remove_inst(blk, idx);
        }
        applied += 1;
    }

    // Phase 4: schedule the surviving bottom expressions (a chained e2 that
    // became a phi is simply not found and skipped).
    for (rw, q) in rewrites.iter().zip(carried.iter()) {
        hoist_bottom_expr(&mut func.blocks[rw.e2_block], rw.e2_dest, *q);
    }
    applied
}

// ── Driver ──────────────────────────────────────────────────────────────────

fn run_round(func: &mut IrFunction, env: &EnvConfig, tm: &TargetModel) -> usize {
    let cfg = CfgAnalysis::build(func);
    let loops = loop_analysis::find_merged_natural_loops(
        func.blocks.len(),
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    );
    if loops.is_empty() {
        return 0;
    }
    // Seed the fresh-id allocator soundly (the cached bound may be 0 or
    // stale; `IrFunction::sound_next_value_id` is the documented contract
    // for passes that synthesize values) and size the flat tables from it.
    let bound = func.sound_next_value_id();
    func.next_value_id = bound;
    let dom = DominanceChecker::new(func.blocks.len(), &cfg.idom);
    let defs = Defs::build(func, bound as usize);

    let mut rewrites: Vec<Rewrite> = Vec::new();
    {
        let cx = Ctx { func: &*func, cfg: &cfg, dom: &dom, defs: &defs, tm, env };
        for lp in &loops {
            plan_loop(&cx, lp, &mut rewrites);
        }
    }
    if rewrites.is_empty() {
        return 0;
    }
    // One rewrite per top expression (a structural invariant of the planner;
    // kept as a cheap guard). Deterministic order for reproducible builds.
    rewrites.sort_by_key(|r| r.e_dest.0);
    rewrites.dedup_by_key(|r| r.e_dest.0);
    apply(func, &rewrites, &cfg.idom, &defs, env)
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    let env = EnvConfig::read();
    if env.disabled || func.blocks.is_empty() {
        return 0;
    }
    let tm = TargetModel::detect();
    let mut total = 0;
    for _ in 0..MAX_ROUNDS {
        let n = run_round(func, &env, &tm);
        total += n;
        if n == 0 {
            break;
        }
    }
    total
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::reexports::Terminator;

    #[test]
    fn const_keys_normalize_to_operation_width() {
        let a = const_key(&IrConst::I32(-1), IrType::U32).unwrap();
        let b = const_key(&IrConst::I64(0xFFFF_FFFF), IrType::U32).unwrap();
        assert_eq!(a, b);
        let c = const_key(&IrConst::I64(0xFFFF_FFFF), IrType::I64).unwrap();
        assert_ne!(a, c);
        assert_eq!(
            const_key(&IrConst::Zero, IrType::F64).unwrap(),
            const_key(&IrConst::F64(0.0), IrType::F64).unwrap()
        );
        assert_ne!(
            const_key(&IrConst::F64(-0.0), IrType::F64).unwrap(),
            const_key(&IrConst::F64(0.0), IrType::F64).unwrap()
        );
        assert!(const_key(&IrConst::F64(1.0), IrType::I32).is_none());
    }

    #[test]
    fn expr_key_canonicalizes_commutative_operands() {
        let (x, y) = (OperandKey::Val(7), OperandKey::Con(3));
        assert_eq!(
            ExprKey::new(ExprOp::Bin(IrBinOp::Add), IrType::I32, x, y),
            ExprKey::new(ExprOp::Bin(IrBinOp::Add), IrType::I32, y, x)
        );
        assert_ne!(
            ExprKey::new(ExprOp::Bin(IrBinOp::Sub), IrType::I32, x, y),
            ExprKey::new(ExprOp::Bin(IrBinOp::Sub), IrType::I32, y, x)
        );
    }

    #[test]
    fn folding_respects_unsigned_storage_and_shift_semantics() {
        let c = |v: i64| Operand::Const(IrConst::I64(v));
        // 200 + 100 wraps to 44 in U8 and must be stored zero-extended.
        let r = fold_const_binop(IrBinOp::Add, &c(200), &c(100), IrType::U8).unwrap();
        assert_eq!(r.to_i64(), Some(44));
        let r = fold_const_binop(IrBinOp::Add, &c(250), &c(5), IrType::U8).unwrap();
        assert_eq!(r.to_i64(), Some(255));
        // Logical shift of a U32 stored as a negative I32 shifts 0xFFFF_FFFF.
        let r = fold_const_binop(
            IrBinOp::LShr,
            &Operand::Const(IrConst::I32(-1)),
            &c(4),
            IrType::U32,
        )
        .unwrap();
        assert_eq!(r.to_i64(), Some(0x0FFF_FFFF));
        // Out-of-range shift amounts are not folded.
        assert!(fold_const_binop(IrBinOp::Shl, &c(1), &c(32), IrType::I32).is_none());
        // FP: finite only, exact IEEE result.
        let f = |v: f64| Operand::Const(IrConst::F64(v));
        assert!(matches!(
            fold_const_binop(IrBinOp::Mul, &f(0.0), &f(0.0), IrType::F64),
            Some(IrConst::F64(v)) if v == 0.0
        ));
        assert!(fold_const_binop(IrBinOp::Mul, &f(f64::INFINITY), &f(0.0), IrType::F64).is_none());
        assert!(fold_const_binop(IrBinOp::Mul, &c(3), &c(5), IrType::I128).is_some());
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock { label: BlockId(label), instructions, terminator, source_spans: Vec::new() }
    }

    fn binop(dest: u32, op: IrBinOp, lhs: Operand, rhs: Operand, ty: IrType) -> Instruction {
        Instruction::BinOp { dest: Value(dest), op, lhs, rhs, ty }
    }

    fn val(v: u32) -> Operand {
        Operand::Value(Value(v))
    }

    fn i64c(v: i64) -> Operand {
        Operand::Const(IrConst::I64(v))
    }

    /// The `backedge_pre_int_recurrence.c` shape:
    ///   x = phi(1, x'); y = x*x; x' = x+3; z = x'*x'; acc' = acc + (y ^ (z >> 17))
    fn int_recurrence() -> IrFunction {
        let mut f = IrFunction::new("run".into(), IrType::U64, Vec::new(), false);
        f.blocks.push(block(0, vec![], Terminator::Branch(BlockId(1))));
        f.blocks.push(block(
            1,
            vec![
                Instruction::Phi {
                    dest: Value(1),
                    ty: IrType::U64,
                    incoming: vec![(i64c(1), BlockId(0)), (val(3), BlockId(1))],
                },
                Instruction::Phi {
                    dest: Value(2),
                    ty: IrType::U64,
                    incoming: vec![(i64c(0), BlockId(0)), (val(9), BlockId(1))],
                },
                Instruction::Phi {
                    dest: Value(10),
                    ty: IrType::U64,
                    incoming: vec![(i64c(0), BlockId(0)), (val(11), BlockId(1))],
                },
                binop(4, IrBinOp::Mul, val(1), val(1), IrType::U64),
                binop(3, IrBinOp::Add, val(1), i64c(3), IrType::U64),
                binop(5, IrBinOp::Mul, val(3), val(3), IrType::U64),
                binop(6, IrBinOp::LShr, val(5), i64c(17), IrType::U64),
                binop(7, IrBinOp::Xor, val(4), val(6), IrType::U64),
                binop(9, IrBinOp::Add, val(2), val(7), IrType::U64),
                binop(11, IrBinOp::Add, val(10), i64c(1), IrType::U64),
                Instruction::Cmp {
                    dest: Value(12),
                    op: crate::ir::reexports::IrCmpOp::Ult,
                    lhs: val(11),
                    rhs: i64c(1000),
                    ty: IrType::U64,
                },
            ],
            Terminator::CondBranch { cond: val(12), true_label: BlockId(1), false_label: BlockId(2) },
        ));
        f.blocks.push(block(
            2,
            vec![binop(13, IrBinOp::Xor, val(9), val(3), IrType::U64)],
            Terminator::Return(Some(val(13))),
        ));
        f.next_value_id = 0; // exercise the sound re-seeding path
        f
    }

    fn count_muls(f: &IrFunction) -> usize {
        f.blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| matches!(i, Instruction::BinOp { op: IrBinOp::Mul, .. }))
            .count()
    }

    #[test]
    fn integer_square_is_carried_across_the_backedge() {
        let mut f = int_recurrence();
        crate::common::types::set_target_elf_machine(EM_X86_64);
        let n = run(&mut f);
        assert_eq!(n, 1, "exactly the top square is rewritten");
        // 1*1 folds to a constant: no preheader instruction, one multiply left.
        assert!(f.blocks[0].instructions.is_empty());
        assert_eq!(count_muls(&f), 1);
        let header = &f.blocks[1];
        let q = header
            .instructions
            .iter()
            .find_map(|i| match i {
                Instruction::Phi { dest, incoming, .. }
                    if incoming.iter().any(|(o, _)| *o == val(5)) =>
                {
                    Some(*dest)
                }
                _ => None,
            })
            .expect("carried phi q = phi(1, z)");
        let q_phi = header.instructions.iter().find(|i| i.dest() == Some(q)).unwrap();
        let Instruction::Phi { incoming, .. } = q_phi else { unreachable!() };
        assert!(incoming.contains(&(Operand::Const(IrConst::I64(1)), BlockId(0))));
        // v4 is gone and every former use reads q.
        assert!(header.instructions.iter().all(|i| i.dest() != Some(Value(4))));
        let xor = header
            .instructions
            .iter()
            .find(|i| i.dest() == Some(Value(7)))
            .unwrap();
        let Instruction::BinOp { lhs, .. } = xor else { unreachable!() };
        assert_eq!(*lhs, Operand::Value(q));
        // Phis stay a prefix and ids are fresh.
        assert!(first_non_phi(header) == 4);
        assert!(q.0 >= 14 && f.next_value_id > q.0);
        // Idempotent: a second run finds nothing new.
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn singly_used_fp_square_is_left_alone_by_default() {
        let mut f = int_recurrence();
        for b in &mut f.blocks {
            for i in &mut b.instructions {
                if let Instruction::BinOp { ty, .. } | Instruction::Phi { ty, .. } = i {
                    *ty = IrType::F64;
                }
            }
        }
        std::env::remove_var("CCC_BEPRE_FP");
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn scale_feeding_add_is_treated_as_absorbed_on_x86() {
        crate::common::types::set_target_elf_machine(EM_X86_64);
        let mut f = IrFunction::new("g".into(), IrType::I64, Vec::new(), false);
        f.blocks.push(block(
            0,
            vec![
                binop(1, IrBinOp::Mul, val(9), i64c(8), IrType::I64),
                binop(2, IrBinOp::Add, val(1), val(10), IrType::I64),
                binop(3, IrBinOp::Mul, val(9), i64c(7), IrType::I64),
                binop(4, IrBinOp::Add, val(3), val(10), IrType::I64),
            ],
            Terminator::Return(Some(val(2))),
        ));
        let defs = Defs::build(&f, 16);
        let tm = TargetModel::detect();
        assert!(absorbed_by_consumer(&f, &defs, 1, &tm));
        assert!(!absorbed_by_consumer(&f, &defs, 3, &tm), "x*7 is a real multiply on x86");
    }
}
