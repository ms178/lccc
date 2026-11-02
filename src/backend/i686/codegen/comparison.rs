//! I686Codegen: comparison operations (float, int, fused branches, select).

use super::emit::{I686Codegen, phys_reg_name};
use crate::backend::state::SlotAddr;
use crate::backend::traits::ArchCodegen;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Instruction, IrCmpOp, IrConst, IrFunction, Operand, Value};

fn reg_for_type(reg: &str, ty: IrType) -> Option<&'static str> {
    match ty {
        IrType::I8 | IrType::U8 => match reg {
            "eax" => Some("al"),
            "ebx" => Some("bl"),
            "ecx" => Some("cl"),
            "edx" => Some("dl"),
            _ => None,
        },
        IrType::I16 | IrType::U16 => match reg {
            "eax" => Some("ax"),
            "ebx" => Some("bx"),
            "ecx" => Some("cx"),
            "edx" => Some("dx"),
            "esi" => Some("si"),
            "edi" => Some("di"),
            "ebp" => Some("bp"),
            _ => None,
        },
        _ => match reg {
            "eax" => Some("eax"),
            "ebx" => Some("ebx"),
            "ecx" => Some("ecx"),
            "edx" => Some("edx"),
            "esi" => Some("esi"),
            "edi" => Some("edi"),
            "ebp" => Some("ebp"),
            _ => None,
        },
    }
}

/// The immediate of an integer compare's constant operand.  A compare runs
/// at <= 32 bits here, where the staging it replaces (`movl $v, %ecx`)
/// means the low 32 bits of `v`; an `I64` constant in `[i32::MIN, u32::MAX]`
/// (e.g. `0x80000000` against a `U32`) is that same 32-bit pattern, so it
/// is an exact `$imm32` instead of a dead `%ecx` load.  Wider constants keep
/// the staged path.
fn cmp_imm32(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(IrConst::I64(v)) if *v > i32::MAX as i64 && *v <= u32::MAX as i64 => {
            Some(*v as u32 as i32 as i64)
        }
        _ => I686Codegen::const_as_imm32(op),
    }
}

/// The conditional jump taken when an integer compare holds.
fn int_jcc(op: IrCmpOp) -> &'static str {
    match op {
        IrCmpOp::Eq => "je",
        IrCmpOp::Ne => "jne",
        IrCmpOp::Slt => "jl",
        IrCmpOp::Sle => "jle",
        IrCmpOp::Sgt => "jg",
        IrCmpOp::Sge => "jge",
        IrCmpOp::Ult => "jb",
        IrCmpOp::Ule => "jbe",
        IrCmpOp::Ugt => "ja",
        IrCmpOp::Uge => "jae",
    }
}

fn cmp_mnemonic(ty: IrType) -> &'static str {
    match ty {
        IrType::I8 | IrType::U8 => "cmpb",
        IrType::I16 | IrType::U16 => "cmpw",
        _ => "cmpl",
    }
}

fn test_mnemonic(ty: IrType) -> &'static str {
    match ty {
        IrType::I8 | IrType::U8 => "testb",
        IrType::I16 | IrType::U16 => "testw",
        _ => "testl",
    }
}

/// How a narrow (8/16-bit) integer value's full 32-bit register image is
/// laid out: the low `bits` bits are the value, the rest zero- (`signed ==
/// false`) or sign-extend it.  i686 keeps every scalar in a 32-bit home and
/// only SOME producers canonicalise the upper bits — a narrow `addl` leaves
/// 260 where the U8 value is 4 — so a fact is recorded only where the
/// producing instruction provably writes the extension.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct NarrowImage {
    pub(super) bits: u8,
    pub(super) signed: bool,
}

impl NarrowImage {
    pub(super) fn of_type(ty: IrType) -> Option<Self> {
        match ty {
            IrType::U8 => Some(Self {
                bits: 8,
                signed: false,
            }),
            IrType::I8 => Some(Self {
                bits: 8,
                signed: true,
            }),
            IrType::U16 => Some(Self {
                bits: 16,
                signed: false,
            }),
            IrType::I16 => Some(Self {
                bits: 16,
                signed: true,
            }),
            _ => None,
        }
    }

    /// Is this image the `signed`-extension of its low `width` bits?  An
    /// extension from `bits <= width` bits of the same kind is (extending
    /// twice is extending once); a ZERO extension from strictly fewer bits
    /// is also a SIGN extension at `width` (bit `width-1` is zero).
    pub(super) fn extends_at(self, width: u8, signed: bool) -> bool {
        self.bits <= width && (self.signed == signed || (!self.signed && self.bits < width))
    }
}

/// Per-function canonical-image facts for narrow integer values.
///
/// Seeds: narrow `Load`s — every i686 load path selects its instruction via
/// `mov_load_for_type` (`movzbl`/`movsbl`/`movzwl`/`movswl`), and
/// `generate_load` emits exactly one load per IR load — and narrow-result
/// `Cast`s, see [`cast_image`].  Propagation: `Copy`, `Phi` and identity
/// `Cast`s of `Value`s — i686 moves scalars with full-width `movl`
/// (register↔register, `store_eax_to`, `operand_to_eax`/`_ecx` slot
/// reloads), so a copy carries its source's image verbatim.  Everything else
/// (narrow ALU results, constants, parameters, calls, inline asm — whose
/// narrow outputs may be stored with `movb`/`movw`) is bottom: no fact.
///
/// Optimistic fixpoint (start loop-carried copies/phis at ⊤, lower to the
/// meet of their sources, monotone ⇒ terminates): sound because every
/// runtime value flowing around a copy/phi cycle entered it through a
/// non-copy producer, which is either a seed with the same image or bottom.
/// Values left at ⊤ (fed only by the cycle itself) are dropped.
/// Image written by the i686 lowering of a non-identity integer-result
/// `Cast`, keyed on the same `classify_cast` the emitter dispatches on
/// (`emit_cast_instrs_impl`; the pair and x87 paths in `emit_cast_impl`
/// end in the same `cast_normalize_eax(to_ty)`), and every result leaves
/// through the full-width `store_eax_to`:
/// * `IntNarrow`, same-size sign changes, `FloatTo{Signed,Unsigned}`
///   normalise to `to_ty`;
/// * `IntWiden` normalises to `from_ty`, re-normalising to `to_ty` only for
///   a signed source into an unsigned destination.
///
/// Any other kind (notably the `Void`/zero-size `Noop`) has no fact.
fn cast_image(from_ty: IrType, to_ty: IrType) -> Option<NarrowImage> {
    use crate::backend::cast::{CastKind, classify_cast};
    let to = NarrowImage::of_type(to_ty)?;
    match classify_cast(from_ty, to_ty) {
        CastKind::IntNarrow { .. }
        | CastKind::SignedToUnsignedSameSize { .. }
        | CastKind::UnsignedToSignedSameSize { .. }
        | CastKind::FloatToSigned { .. }
        | CastKind::FloatToUnsigned { .. } => Some(to),
        CastKind::IntWiden { .. } => match NarrowImage::of_type(from_ty) {
            Some(from) if !(from.signed && !to.signed) => Some(from),
            _ => Some(to),
        },
        _ => None,
    }
}

pub(super) fn compute_narrow_images(func: &IrFunction) -> FxHashMap<u32, NarrowImage> {
    #[derive(Clone, Copy, PartialEq)]
    enum Lat {
        Top,
        Img(NarrowImage),
        Bottom,
    }
    let mut lat: FxHashMap<u32, Lat> = FxHashMap::default();
    let mut flow: Vec<(u32, Vec<Option<u32>>)> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { dest, ty, .. } => {
                    if let Some(img) = NarrowImage::of_type(*ty) {
                        lat.insert(dest.0, Lat::Img(img));
                    }
                }
                Instruction::Cast {
                    dest,
                    src,
                    from_ty,
                    to_ty,
                } => {
                    if from_ty == to_ty {
                        // `CastKind::Noop`: operand_to_eax + store_eax_to,
                        // a full-width copy of the source image.
                        let s = match src {
                            Operand::Value(v) => Some(v.0),
                            Operand::Const(_) => None,
                        };
                        lat.insert(dest.0, Lat::Top);
                        flow.push((dest.0, vec![s]));
                    } else if let Some(img) = cast_image(*from_ty, *to_ty) {
                        lat.insert(dest.0, Lat::Img(img));
                    }
                }
                Instruction::Copy { dest, src } => {
                    let s = match src {
                        Operand::Value(v) => Some(v.0),
                        Operand::Const(_) => None,
                    };
                    lat.insert(dest.0, Lat::Top);
                    flow.push((dest.0, vec![s]));
                }
                Instruction::Phi { dest, incoming, .. } => {
                    let srcs = incoming
                        .iter()
                        .map(|(op, _)| match op {
                            Operand::Value(v) => Some(v.0),
                            Operand::Const(_) => None,
                        })
                        .collect();
                    lat.insert(dest.0, Lat::Top);
                    flow.push((dest.0, srcs));
                }
                _ => {}
            }
        }
    }
    // A value defined twice (not SSA-shaped at this point) is never trusted.
    let mut defs: FxHashMap<u32, u32> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(d) = inst.dest() {
                *defs.entry(d.0).or_insert(0) += 1;
            }
        }
    }
    for (v, n) in &defs {
        if *n > 1 {
            lat.insert(*v, Lat::Bottom);
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for (dest, srcs) in &flow {
            if lat.get(dest) == Some(&Lat::Bottom) {
                continue;
            }
            let mut meet = Lat::Top;
            for s in srcs {
                let l = match s {
                    Some(v) => lat.get(v).copied().unwrap_or(Lat::Bottom),
                    None => Lat::Bottom,
                };
                meet = match (meet, l) {
                    (Lat::Top, x) | (x, Lat::Top) => x,
                    (Lat::Img(a), Lat::Img(b)) if a == b => Lat::Img(a),
                    _ => Lat::Bottom,
                };
                if meet == Lat::Bottom {
                    break;
                }
            }
            if lat.get(dest) != Some(&meet) {
                lat.insert(*dest, meet);
                changed = true;
            }
        }
    }
    lat.into_iter()
        .filter_map(|(v, l)| match l {
            Lat::Img(img) => Some((v, img)),
            _ => None,
        })
        .collect()
}

/// The operand-image kind under which a FULL 32-bit compare computes the
/// same flags-reader outcome as the `ty`-width compare for predicate `op`:
/// `Some(false)` zero-extended, `Some(true)` sign-extended, `None` when the
/// narrow compare is required.  Both images must be the chosen extension at
/// the compare width: then the 32-bit operands are the mathematically
/// extended narrow values, and zero extension preserves unsigned order,
/// sign extension preserves signed order, and either preserves equality.
/// `rhs == None` with `rhs_is_imm` means an immediate (normalised by
/// `wide_cmp_imm` to the chosen extension, so it is always admissible).
pub(super) fn wide_cmp_kind(
    op: IrCmpOp,
    ty: IrType,
    lhs: Option<NarrowImage>,
    rhs: Option<NarrowImage>,
    rhs_is_imm: bool,
) -> Option<bool> {
    let width = match ty {
        IrType::I8 | IrType::U8 => 8,
        IrType::I16 | IrType::U16 => 16,
        _ => return None,
    };
    let kinds: &[bool] = match op {
        IrCmpOp::Eq | IrCmpOp::Ne => &[false, true],
        IrCmpOp::Ult | IrCmpOp::Ule | IrCmpOp::Ugt | IrCmpOp::Uge => &[false],
        IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge => &[true],
    };
    let l = lhs?;
    kinds.iter().copied().find(|&signed| {
        l.extends_at(width, signed)
            && (rhs_is_imm || rhs.is_some_and(|r| r.extends_at(width, signed)))
    })
}

/// The immediate as the `signed`-extension of its low `ty` bits — the same
/// representation `wide_cmp_kind` proved for the register operand.
fn wide_cmp_imm(imm: i64, ty: IrType, signed: bool) -> i64 {
    match (ty, signed) {
        (IrType::I8 | IrType::U8, false) => imm as u8 as i64,
        (IrType::I8 | IrType::U8, true) => imm as i8 as i64,
        (IrType::I16 | IrType::U16, false) => imm as u16 as i64,
        (IrType::I16 | IrType::U16, true) => imm as i16 as i64,
        _ => imm as i32 as i64,
    }
}

impl I686Codegen {
    /// Register name for a scalar value safe to compare in place (not an
    /// alloca address, not a wide pair). None for slot-resident values and
    /// immediates.
    fn cmp_operand_reg(&self, op: &Operand) -> Option<&'static str> {
        if let Operand::Value(v) = op {
            if self.state.is_alloca(v.0)
                || self.state.wide_values.contains(&v.0)
                || self.state.f128_direct_slots.contains(&v.0)
            {
                return None;
            }
            if let Some(&phys) = self.reg_assignments.get(&v.0) {
                return Some(phys_reg_name(phys));
            }
        }
        None
    }

    /// The canonical image of a `Value` operand (None for constants and for
    /// values with no proven image — see `compute_narrow_images`).
    fn narrow_image_of(&self, op: &Operand) -> Option<NarrowImage> {
        match op {
            Operand::Value(v) => self.narrow_images.get(&v.0).copied(),
            Operand::Const(_) => None,
        }
    }

    /// `Some(signed)` when the compare may run at 32 bits (see
    /// `wide_cmp_kind`); the rhs counts as an immediate only when it IS one.
    fn wide_cmp_ok(&self, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) -> Option<bool> {
        let rhs_is_imm = cmp_imm32(rhs).is_some();
        wide_cmp_kind(
            op,
            ty,
            self.narrow_image_of(lhs),
            self.narrow_image_of(rhs),
            rhs_is_imm,
        )
    }

    /// Emit the flag-setting compare for an integer compare, staging the LHS
    /// through the accumulator unless it is register-resident (in which case
    /// `testl %R,%R` / `cmpl $imm,%R` / `cmpl %R,%L` is emitted in place and
    /// %eax stays untouched). `%ecx` is the slot-operand scratch, so a LHS in
    /// %ecx never takes the direct path against a slot-resident RHS.
    ///
    /// A narrow compare whose register has no sub-register of the compare
    /// width (`%esi`/`%edi`/`%ebp` at byte width) runs at 32 bits when both
    /// operand images are proven extensions (`wide_cmp_ok`) — the exact
    /// compare without staging through `%eax`; otherwise the caller stages.
    fn emit_int_cmp_flags_direct(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) -> bool {
        let Some(lreg_32) = self.cmp_operand_reg(lhs) else {
            return false;
        };
        let Some(lreg) = reg_for_type(lreg_32, ty) else {
            return self.emit_int_cmp_flags_wide(op, lreg_32, lhs, rhs, ty);
        };
        let cmp_op = cmp_mnemonic(ty);
        let test_op = test_mnemonic(ty);

        if let Some(imm) = cmp_imm32(rhs) {
            let imm = Self::normalize_cmp_imm(imm, ty);
            if imm == 0 {
                emit!(self.state, "    {} %{}, %{}", test_op, lreg, lreg);
            } else {
                emit!(self.state, "    {} ${}, %{}", cmp_op, imm, lreg);
            }
        } else if let Some(rreg) = self.cmp_operand_reg(rhs).and_then(|r| reg_for_type(r, ty)) {
            emit!(self.state, "    {} %{}, %{}", cmp_op, rreg, lreg);
        } else {
            // A register rhs without the sub-register (`%esi`/`%edi`/`%ebp`
            // at byte width) under a proven-extension pair: one `cmpl`.
            if self.cmp_operand_reg(rhs).is_some()
                && self.emit_int_cmp_flags_wide(op, lreg_32, lhs, rhs, ty)
            {
                return true;
            }
            // Otherwise (a slot rhs, or that register without the proof)
            // stage ONLY the rhs through %ecx (`movl %esi,%ecx; cmpb
            // %cl,%dl`), never both operands through the accumulator.
            if lreg_32 == "ecx" {
                return false; // staging rhs into %ecx would clobber lhs
            }
            let Some(ecx_sub) = reg_for_type("ecx", ty) else {
                return false;
            };
            self.operand_to_ecx(rhs);
            emit!(self.state, "    {} %{}, %{}", cmp_op, ecx_sub, lreg);
        }
        true
    }

    /// The 32-bit form of a narrow compare with a register-resident LHS
    /// `lreg_32`, emitted only under a `wide_cmp_ok` proof.  Returns false
    /// (nothing emitted) when the proof fails or the slot rhs would need the
    /// `%ecx` scratch the LHS occupies.
    fn emit_int_cmp_flags_wide(
        &mut self,
        op: IrCmpOp,
        lreg_32: &'static str,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) -> bool {
        let Some(signed) = self.wide_cmp_ok(op, lhs, rhs, ty) else {
            return false;
        };
        if let Some(imm) = cmp_imm32(rhs) {
            let imm = wide_cmp_imm(imm, ty, signed);
            if imm == 0 {
                emit!(self.state, "    testl %{}, %{}", lreg_32, lreg_32);
            } else {
                emit!(self.state, "    cmpl ${}, %{}", imm, lreg_32);
            }
        } else if let Some(rreg_32) = self.cmp_operand_reg(rhs) {
            emit!(self.state, "    cmpl %{}, %{}", rreg_32, lreg_32);
        } else {
            if lreg_32 == "ecx" {
                return false;
            }
            // Full-width slot reload: the stored image is the proven one.
            self.operand_to_ecx(rhs);
            emit!(self.state, "    cmpl %ecx, %{}", lreg_32);
        }
        true
    }

    /// A narrow compare whose LHS is a slot-resident VALUE (a compiler
    /// temporary: never an alloca, so never a volatile object) read nowhere
    /// else (`value_use_counts == 1`) and not cached in `%eax`: compare the
    /// slot in place.  `operand_to_eax` would read that very slot now
    /// (`movl slot,%eax`), and the low byte/word of the slot is the narrow
    /// value (little-endian), so `cmp{b,w} $I|%r, slot` is the identical
    /// compare minus the load; the only thing lost is the `%eax` cache entry
    /// the staged load parks, which a single-use value never consumes.  A
    /// register rhs without the sub-register needs the `wide_cmp_ok` proof
    /// (the slot holds the full `movl` image of its producer).  Returns false,
    /// emitting nothing, for every other shape.
    fn emit_int_cmp_flags_slot(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) -> bool {
        if !matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16) {
            return false;
        }
        let Operand::Value(lv) = lhs else {
            return false;
        };
        let Some(slot) = self.temp_slot_of(lv) else {
            return false;
        };
        if self.state.reg_cache.acc_has(lv.0, false)
            || self.state.value_use_counts.get(lv.0 as usize).copied() != Some(1)
        {
            return false;
        }
        let cmp_op = cmp_mnemonic(ty);
        if let Some(imm) = cmp_imm32(rhs) {
            let imm = Self::normalize_cmp_imm(imm, ty);
            let sr = self.slot_ref(slot);
            emit!(self.state, "    {} ${}, {}", cmp_op, imm, sr);
            return true;
        }
        let Operand::Value(rv) = rhs else {
            return false;
        };
        let Some(r32) = self.direct_reg_src_ref(rv) else {
            return false;
        };
        if let Some(rsub) = reg_for_type(r32.trim_start_matches('%'), ty) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    {} %{}, {}", cmp_op, rsub, sr);
            return true;
        }
        if self.wide_cmp_ok(op, lhs, rhs, ty).is_some() {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    cmpl {}, {}", r32, sr);
            return true;
        }
        false
    }

    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        if ty == IrType::F64 {
            let swap = matches!(
                op,
                IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
            );
            let (first, second) = if swap { (lhs, rhs) } else { (rhs, lhs) };
            self.emit_f64_load_to_x87(first);
            self.emit_f64_load_to_x87(second);
            self.state.emit("    fucomip %st(1), %st");
            self.state.emit("    fstp %st(0)");

            match op {
                IrCmpOp::Eq => {
                    self.state.emit("    setnp %al");
                    self.state.emit("    sete %cl");
                    self.state.emit("    andb %cl, %al");
                }
                IrCmpOp::Ne => {
                    self.state.emit("    setp %al");
                    self.state.emit("    setne %cl");
                    self.state.emit("    orb %cl, %al");
                }
                IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                    self.state.emit("    seta %al");
                }
                IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                    self.state.emit("    setae %al");
                }
            }
            self.state.emit("    movzbl %al, %eax");
            self.state.reg_cache.invalidate_acc();
            self.store_eax_to(dest);
            return;
        }
        let swap_operands = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        if self.sse2_scalar_enabled {
            // F32 with SSE2: UCOMISS on the `movd`-transferred bits.
            let (first, second) = if swap_operands {
                (rhs, lhs)
            } else {
                (lhs, rhs)
            };
            self.operand_to_eax(first);
            self.state.emit("    movd %eax, %xmm0");
            self.operand_to_ecx(second);
            self.state.emit("    movd %ecx, %xmm1");
            self.state.emit("    ucomiss %xmm1, %xmm0");
        } else {
            // F32 without SSE2: the F64 x87 shape. FLDS widens exactly,
            // so FUCOMIP orders the F32 values identically to UCOMISS
            // (unordered sets ZF=PF=CF=1, quiet NaNs raise nothing).
            let (first, second) = if swap_operands {
                (lhs, rhs)
            } else {
                (rhs, lhs)
            };
            self.emit_f32_load_to_x87(first);
            self.emit_f32_load_to_x87(second);
            self.state.emit("    fucomip %st(1), %st");
            self.state.emit("    fstp %st(0)");
        }

        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        let swap = matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sle | IrCmpOp::Ule
        );
        let (first, second) = if swap { (lhs, rhs) } else { (rhs, lhs) };
        self.emit_f128_load_to_x87(first);
        self.emit_f128_load_to_x87(second);
        self.state.emit("    fucomip %st(1), %st");
        self.state.emit("    fstp %st(0)");

        match op {
            IrCmpOp::Eq => {
                self.state.emit("    setnp %al");
                self.state.emit("    sete %cl");
                self.state.emit("    andb %cl, %al");
            }
            IrCmpOp::Ne => {
                self.state.emit("    setp %al");
                self.state.emit("    setne %cl");
                self.state.emit("    orb %cl, %al");
            }
            IrCmpOp::Slt | IrCmpOp::Ult | IrCmpOp::Sgt | IrCmpOp::Ugt => {
                self.state.emit("    seta %al");
            }
            IrCmpOp::Sle | IrCmpOp::Ule | IrCmpOp::Sge | IrCmpOp::Uge => {
                self.state.emit("    setae %al");
            }
        }
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Normalize a compare immediate to the OPERAND's width and signedness.
    ///
    /// lhs reaches %eax via movzbl/movswl/movsbl (operand_to_eax extends
    /// sub-int loads to 32 bits), so the 32-bit compare is exact ONLY when
    /// the immediate is represented the same way the extension represents
    /// the operand. `(unsigned char)255 == (char)-1` miscompiled to
    /// `cmpl $-1, %eax` against a zero-extended 255 (i686-only; x86-64 uses
    /// width-matched cmpb). Truncate to the type's width, then zero- or
    /// sign-extend by the type's signedness — matching operand_to_eax.
    fn normalize_cmp_imm(imm: i64, ty: IrType) -> i64 {
        match ty {
            IrType::I8 => imm as i8 as i64,
            IrType::U8 => imm as u8 as i64,
            IrType::I16 => imm as i16 as i64,
            IrType::U16 => imm as u16 as i64,
            // I32/U32/Ptr compare as full 32-bit; U32 immediates keep their
            // bit pattern under the i32 print (cmpl is width-exact).
            _ => imm as i32 as i64,
        }
    }

    /// Emit `cmp{b,w,l} $imm, (ptr)` staging the pointer exactly like an
    /// indirect load. Returns false when the pointer has no resolvable
    /// slot/register home (the caller re-materializes the skipped load).
    pub(super) fn emit_mem_cmp_imm(&mut self, ptr: u32, ty: IrType, imm: i64) -> bool {
        let mnemonic = match ty {
            IrType::I8 | IrType::U8 => "cmpb",
            IrType::I16 | IrType::U16 => "cmpw",
            IrType::I32 | IrType::U32 | IrType::Ptr => "cmpl",
            _ => return false,
        };
        let Some(addr) = self.state.resolve_slot_addr(ptr) else {
            return false;
        };
        match addr {
            SlotAddr::Direct(slot) => {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    {} ${}, {}", mnemonic, imm, sr);
            }
            SlotAddr::Indirect(slot) => {
                self.emit_load_ptr_from_slot(slot, ptr);
                emit!(self.state, "    {} ${}, (%ecx)", mnemonic, imm);
            }
            SlotAddr::OverAligned(slot, id) => {
                self.emit_alloca_aligned_addr(slot, id);
                emit!(self.state, "    {} ${}, (%ecx)", mnemonic, imm);
            }
            SlotAddr::Reg(reg) => emit!(
                self.state,
                "    {} ${}, (%{})",
                mnemonic,
                imm,
                phys_reg_name(reg)
            ),
        }
        true
    }

    /// Consume a pending load→memory-compare fold (see
    /// `generation::detect_load_cmp_mem_fold`): when `lhs` is a Load whose
    /// single use is this `Eq`/`Ne`-vs-imm compare, emit
    /// `cmp{b,w,l} $imm,(mem)` and report success so the caller skips the
    /// register-resident compare.
    fn take_load_cmp_fold(&mut self, lhs: &Operand, rhs: &Operand, op: IrCmpOp) -> bool {
        let Operand::Value(lv) = lhs else {
            return false;
        };
        let Some((ptr, lty, imm)) = self.state.pending_load_cmp.remove(&lv.0) else {
            return false;
        };
        if !matches!(op, IrCmpOp::Eq | IrCmpOp::Ne)
            || crate::backend::generation::cmp_fold_imm(rhs, lty) != Some(imm)
        {
            // Detection guarantees this cannot happen; re-materialize the load
            // so the compare sees a real value instead of garbage.
            crate::backend::traits::emit_load_default(self, &Value(lv.0), &Value(ptr), lty);
            return false;
        }
        if !self.emit_mem_cmp_imm(ptr, lty, imm) {
            // Pointer is not resolvable (e.g. a global address): fall back to
            // the materialized load + normal compare.
            crate::backend::traits::emit_load_default(self, &Value(lv.0), &Value(ptr), lty);
            return false;
        }
        true
    }

    /// Set the flags for `lhs <op> rhs` at width `ty` through the
    /// accumulator: `lhs` is staged in `%eax` and compared at ITS width
    /// (`cmp{b,w,l}` / `test{b,w,l}` on `%al`/`%ax`/`%eax`).  The width is
    /// load-bearing: a narrow IR value lives in a 32-bit register with
    /// unspecified upper bits (`addl` of two U8 lanes leaves 260 where the
    /// value is 4), so a 32-bit compare of a U8 would read garbage.  The
    /// single implementation behind both the value compare and the fused
    /// compare-and-branch -- the branch path once carried its own copy that
    /// hard-coded `cmpl`/`testl` and branched on the unnormalised bits.
    fn emit_int_cmp_flags_staged(&mut self, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) {
        if self.emit_int_cmp_flags_slot(op, lhs, rhs, ty) {
            return;
        }
        // A proven-extension pair (`wide_cmp_kind`) makes the 32-bit compare
        // exact.  It is chosen where it is no larger than the narrow form and
        // keeps the peephole's SAME-WIDTH memory folds available (those never
        // change an access width, so they need no volatility proof):
        //   * a LHS reloaded from its slot: `movl S,%eax; cmpl $I,%eax` folds
        //     to `cmpl $I,S` once %eax is dead (`cmpb $I,%al` cannot: the
        //     fold would narrow a 4-byte load);
        //   * a slot RHS: `movl S,%ecx; cmpl %ecx,%eax` folds to `cmpl S,%eax`;
        //   * a register RHS without the sub-register: `cmpl %esi,%eax`
        //     instead of `movl %esi,%ecx; cmpb %cl,%al`.
        // A LHS already cached in %eax keeps `cmpb $I,%al` (a byte shorter
        // than `cmpl $I,%eax`, and nothing to fold).
        let wide = self.wide_cmp_ok(op, lhs, rhs, ty);
        let lhs_reloads = match lhs {
            Operand::Value(lv) => {
                self.temp_slot_of(lv).is_some() && !self.state.reg_cache.acc_has(lv.0, false)
            }
            Operand::Const(_) => false,
        };
        self.operand_to_eax(lhs);
        let eax_sub = reg_for_type("eax", ty).unwrap_or("eax");
        let cmp_op = cmp_mnemonic(ty);
        let test_op = test_mnemonic(ty);

        // Constant rhs: compare against the immediate directly instead of
        // staging it in %ecx (keeps %ecx clean across fused compares).
        if let Some(imm) = cmp_imm32(rhs) {
            let (op32, test32, sub, imm) = match wide {
                Some(signed) if lhs_reloads => {
                    ("cmpl", "testl", "eax", wide_cmp_imm(imm, ty, signed))
                }
                _ => (cmp_op, test_op, eax_sub, Self::normalize_cmp_imm(imm, ty)),
            };
            if imm == 0 {
                emit!(self.state, "    {} %{}, %{}", test32, sub, sub);
            } else {
                emit!(self.state, "    {} ${}, %{}", op32, imm, sub);
            }
            return;
        }
        let Operand::Value(rv) = rhs else {
            // Unreachable for integer operands (every constant is an imm32),
            // kept total: stage through %ecx at the compare width.
            let ecx_sub = reg_for_type("ecx", ty).unwrap_or("ecx");
            self.operand_to_ecx(rhs);
            emit!(self.state, "    {} %{}, %{}", cmp_op, ecx_sub, eax_sub);
            return;
        };
        // A REGISTER-resident rhs compares in place when it has a sub-register
        // of the compare width (`%esi`/`%edi`/`%ebp` have no byte form on
        // i686); the read is identical to the `movl %reg,%ecx` staging it
        // replaces.  Without the sub-register: `cmpl` under the proof.
        if let Some(r32) = self.direct_reg_src_ref(rv) {
            if let Some(r) = reg_for_type(r32.trim_start_matches('%'), ty) {
                emit!(self.state, "    {} %{}, %{}", cmp_op, r, eax_sub);
                return;
            }
            if wide.is_some() {
                emit!(self.state, "    cmpl {}, %eax", r32);
                return;
            }
        } else if wide.is_some() {
            // Slot (or cached) rhs under the proof: the staged 32-bit form
            // the same-width peephole fold turns into `cmpl S,%eax`.
            self.operand_to_ecx(rhs);
            self.state.emit("    cmpl %ecx, %eax");
            return;
        } else if let Some(slot) = self.temp_slot_of(rv) {
            // A single-use slot-resident VALUE rhs not cached in %ecx:
            // compare its slot directly (`operand_to_ecx` would read that
            // very slot now; its low byte/word is the narrow value).
            if !self.state.reg_cache.sec_has(rv.0, false)
                && self.state.value_use_counts.get(rv.0 as usize).copied() == Some(1)
            {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    {} {}, %{}", cmp_op, sr, eax_sub);
                return;
            }
        }
        let ecx_sub = reg_for_type("ecx", ty).unwrap_or("ecx");
        self.operand_to_ecx(rhs);
        emit!(self.state, "    {} %{}, %{}", cmp_op, ecx_sub, eax_sub);
    }

    /// The stack slot of a slot-resident scalar VALUE — a compiler
    /// temporary (never an alloca, so never a volatile object; never a wide
    /// pair or an x87 direct slot).  `None` for register-homed values,
    /// allocas, home-less accumulator-flow values and wide carriers.
    fn temp_slot_of(&self, v: &Value) -> Option<crate::backend::state::StackSlot> {
        if self.state.is_alloca(v.0)
            || self.state.wide_values.contains(&v.0)
            || self.state.f128_direct_slots.contains(&v.0)
            || self.reg_assignments.contains_key(&v.0)
        {
            return None;
        }
        self.state.get_slot(v.0)
    }

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Load→memory-compare fold: `movsbl (mem),%r; testl %r,%r` collapses
        // to `cmpb $0,(mem)`, freeing the register and the push/pop pair.
        let folded = self.take_load_cmp_fold(lhs, rhs, op);
        // Accumulator-bypass: compare a register-resident LHS in place
        // (`testl %R,%R` / `cmpl $imm,%R` / `cmpl %R,%L`) instead of the
        // `movl %R,%eax; cmpl …, %eax` round-trip.
        if !folded && !self.emit_int_cmp_flags_direct(op, lhs, rhs, ty) {
            self.emit_int_cmp_flags_staged(op, lhs, rhs, ty);
        }

        let set_instr = match op {
            IrCmpOp::Eq => "sete",
            IrCmpOp::Ne => "setne",
            IrCmpOp::Slt => "setl",
            IrCmpOp::Sle => "setle",
            IrCmpOp::Sgt => "setg",
            IrCmpOp::Sge => "setge",
            IrCmpOp::Ult => "setb",
            IrCmpOp::Ule => "setbe",
            IrCmpOp::Ugt => "seta",
            IrCmpOp::Uge => "setae",
        };
        emit!(self.state, "    {} %al", set_instr);
        self.state.emit("    movzbl %al, %eax");
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Fused BitTest→CondBranch: `btl index, base; jc/jnc`. CF holds the
    /// selected bit; nothing is materialized. Mirrors the x86-64 hook —
    /// only 32-bit masks reach this target (legality gates in simplify /
    /// set_membership).
    pub(super) fn emit_fused_bit_test_branch_blocks_impl(
        &mut self,
        base: &Operand,
        index: &Operand,
        _ty: IrType,
        true_block: crate::ir::reexports::BlockId,
        false_block: crate::ir::reexports::BlockId,
    ) {
        // Stage the BASE (mask) — register home preferred, else %eax.
        let base_ref: String = match base {
            Operand::Value(v) => match self.direct_reg_src_ref(v) {
                Some(r) => r,
                None => {
                    self.operand_to_eax(base);
                    "%eax".to_string()
                }
            },
            Operand::Const(_) => {
                self.operand_to_eax(base);
                "%eax".to_string()
            }
        };
        let const_index = match index {
            Operand::Const(c) => c.to_i64().filter(|v| *v >= 0),
            _ => None,
        };
        if let Some(imm) = const_index {
            let bit = (imm as u32) % 32;
            emit!(self.state, "    btl ${}, {}", bit, base_ref);
        } else {
            // Variable index: register home or %ecx staging. %ecx is safe —
            // the base staging above uses %eax only.
            let idx_ref: String = match index {
                Operand::Value(v) => match self.direct_reg_src_ref(v) {
                    Some(r) => r,
                    None => {
                        self.operand_to_ecx(index);
                        "%ecx".to_string()
                    }
                },
                _ => {
                    self.operand_to_ecx(index);
                    "%ecx".to_string()
                }
            };
            emit!(self.state, "    btl {}, {}", idx_ref, base_ref);
        }
        let true_label = true_block.as_label();
        let false_label = false_block.as_label();
        emit!(self.state, "    jc {}", true_label);
        emit!(self.state, "    jmp {}", false_label);
        self.state.reg_cache.invalidate_all();
    }

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        // Load→memory-compare fold (see emit_int_cmp_impl): the branch path
        // fuses the Cmp into the CondBranch terminator, so the fold must be
        // taken here too or the skipped load never materializes.
        let folded = self.take_load_cmp_fold(lhs, rhs, op);
        // Accumulator-bypass: compare a register-resident LHS in place; the
        // direct flag emitter leaves %eax untouched (see emit_int_cmp_impl).
        if !folded && !self.emit_int_cmp_flags_direct(op, lhs, rhs, ty) {
            self.emit_int_cmp_flags_staged(op, lhs, rhs, ty);
        }

        emit!(self.state, "    {} {}", int_jcc(op), true_label);
        emit!(self.state, "    jmp {}", false_label);
        self.state.reg_cache.invalidate_all();
    }

    /// Fused integer Cmp→Select (`supports_fused_cmp_select`): the compare's
    /// flags drive `jCC .Lsel_true` directly — no `setCC`/`movzbl` boolean,
    /// no re-test.  Generation calls this only for an ADJACENT compare (at
    /// most a dead Copy/integer Cast between) whose result is used solely
    /// by this select, and never for float, long-double, 128-bit or (on
    /// this 32-bit target) I64/U64 compares (`detect_cmp_select_fusion`).
    ///
    /// Same flag emitters as the fused branch, so every narrow-compare and
    /// load→memory-compare law carries over unchanged.  Register-allocator
    /// safety: the compare stages through %ecx exactly as the skipped Cmp
    /// would, one program point later.  Adjacency means every value live
    /// across the Select point except its own destination is live across
    /// the Cmp point too, which `collect_i686_scratch_hazard_points` already
    /// keeps out of %ecx; the destination is written only by the arms, after
    /// the compare has read its operands.  %edx is touched by neither.
    pub(super) fn emit_fused_cmp_select_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        cmp_ty: IrType,
        true_val: &Operand,
        false_val: &Operand,
        dest: &Value,
        sel_ty: IrType,
    ) {
        assert!(
            cmp_ty.is_integer() && !super::emit::is_pair_scalar(cmp_ty),
            "fused cmp-select reached the i686 backend with a {cmp_ty:?} compare"
        );
        let folded = self.take_load_cmp_fold(lhs, rhs, op);
        if !folded && !self.emit_int_cmp_flags_direct(op, lhs, rhs, cmp_ty) {
            self.emit_int_cmp_flags_staged(op, lhs, rhs, cmp_ty);
        }
        let label_id = self.state.next_label_id();
        let true_label = format!(".Lsel_true_{}", label_id);
        let end_label = format!(".Lsel_end_{}", label_id);
        emit!(self.state, "    {} {}", int_jcc(op), true_label);
        self.emit_select_arms(dest, true_val, false_val, sel_ty, &true_label, &end_label);
    }

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
    ) {
        // Constant-fold wide conditions at compile time
        match cond {
            Operand::Const(IrConst::I64(v)) => {
                self.emit_copy_value(dest, if *v != 0 { true_val } else { false_val });
                return;
            }
            Operand::Const(IrConst::F64(fval)) => {
                self.emit_copy_value(dest, if *fval != 0.0 { true_val } else { false_val });
                return;
            }
            _ => {}
        }

        let cond_is_wide = matches!(cond, Operand::Value(v) if self.state.is_wide_value(v.0));
        let result_is_wide = super::emit::is_pair_scalar(ty);

        if !cond_is_wide && !result_is_wide {
            let label_id = self.state.next_label_id();
            let true_label = format!(".Lsel_true_{}", label_id);
            let end_label = format!(".Lsel_end_{}", label_id);
            self.emit_load_operand(cond);
            self.emit_branch_nonzero(&true_label);
            self.emit_select_arms(dest, true_val, false_val, ty, &true_label, &end_label);
            return;
        }

        let label_id = self.state.next_label_id();
        let true_label = format!(".Lsel_true_{}", label_id);
        let end_label = format!(".Lsel_end_{}", label_id);

        if cond_is_wide {
            if let Operand::Value(v) = cond {
                self.emit_wide_value_to_eax_ored(v.0);
                self.state.reg_cache.invalidate_acc();
            }
        } else {
            self.operand_to_eax(cond);
        }

        self.emit_branch_nonzero(&true_label);
        self.emit_select_arms(dest, true_val, false_val, ty, &true_label, &end_label);
    }

    /// The arms of a branchy select, entered right after the conditional
    /// jump to `true_label`: the false arm falls through, the true arm
    /// follows `true_label`, both meet at `end_label`.
    fn emit_select_arms(
        &mut self,
        dest: &Value,
        true_val: &Operand,
        false_val: &Operand,
        ty: IrType,
        true_label: &str,
        end_label: &str,
    ) {
        if !super::emit::is_pair_scalar(ty) {
            // When dest is register-allocated, materialize each arm DIRECTLY
            // into that register (movl slot,%esi) instead of the
            // load-to-%eax + movl %eax,%esi detour. The peephole cannot fuse
            // these afterwards because the pair sits right before a label
            // (deadness unprovable across the barrier), so the win must
            // happen here. Falls back to the accumulator path per-arm.
            let dest_phys = self.get_phys_reg_for_value(dest.0);
            if let Some(d) = dest_phys {
                if !self.emit_load_direct_to_phys_reg(false_val, d) {
                    self.emit_load_operand(false_val);
                    self.emit_store_result(dest);
                }
            } else {
                self.emit_load_operand(false_val);
                self.emit_store_result(dest);
            }
            self.emit_branch(end_label);
            self.state.emit_fmt(format_args!("{}:", true_label));
            if let Some(d) = dest_phys {
                if !self.emit_load_direct_to_phys_reg(true_val, d) {
                    self.emit_load_operand(true_val);
                    self.emit_store_result(dest);
                }
            } else {
                self.emit_load_operand(true_val);
                self.emit_store_result(dest);
            }
            self.state.emit_fmt(format_args!("{}:", end_label));
            self.state.reg_cache.invalidate_acc();
            return;
        }

        self.emit_copy_value(dest, false_val);
        self.emit_branch(end_label);

        self.state.emit_fmt(format_args!("{}:", true_label));
        self.emit_copy_value(dest, true_val);

        self.state.emit_fmt(format_args!("{}:", end_label));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrConst, Terminator};

    const TYS: [IrType; 4] = [IrType::U8, IrType::I8, IrType::U16, IrType::I16];
    const OPS: [IrCmpOp; 10] = [
        IrCmpOp::Eq,
        IrCmpOp::Ne,
        IrCmpOp::Slt,
        IrCmpOp::Sle,
        IrCmpOp::Sgt,
        IrCmpOp::Sge,
        IrCmpOp::Ult,
        IrCmpOp::Ule,
        IrCmpOp::Ugt,
        IrCmpOp::Uge,
    ];

    fn images() -> Vec<NarrowImage> {
        let mut v = Vec::new();
        for bits in [8u8, 16] {
            for signed in [false, true] {
                v.push(NarrowImage { bits, signed });
            }
        }
        v
    }

    fn width(ty: IrType) -> u32 {
        if matches!(ty, IrType::U8 | IrType::I8) {
            8
        } else {
            16
        }
    }

    /// The 32-bit register image `img` gives the raw `img.bits` pattern `x`.
    fn image(x: u32, img: NarrowImage) -> u32 {
        let m = (1u32 << img.bits) - 1;
        let low = x & m;
        if img.signed && (low >> (img.bits - 1)) & 1 == 1 {
            low | !m
        } else {
            low
        }
    }

    fn holds(op: IrCmpOp, a: i64, b: i64) -> bool {
        match op {
            IrCmpOp::Eq => a == b,
            IrCmpOp::Ne => a != b,
            IrCmpOp::Slt | IrCmpOp::Ult => a < b,
            IrCmpOp::Sle | IrCmpOp::Ule => a <= b,
            IrCmpOp::Sgt | IrCmpOp::Ugt => a > b,
            IrCmpOp::Sge | IrCmpOp::Uge => a >= b,
        }
    }

    fn is_signed(op: IrCmpOp) -> bool {
        matches!(
            op,
            IrCmpOp::Slt | IrCmpOp::Sle | IrCmpOp::Sgt | IrCmpOp::Sge
        )
    }

    /// IR semantics: compare the low `ty` bits of both registers.
    fn narrow_cmp(op: IrCmpOp, ty: IrType, a: u32, b: u32) -> bool {
        let w = width(ty);
        let m = (1u32 << w) - 1;
        let (a, b) = (a & m, b & m);
        if is_signed(op) {
            let sx = |x: u32| ((x << (32 - w)) as i32 >> (32 - w)) as i64;
            holds(op, sx(a), sx(b))
        } else {
            holds(op, a as i64, b as i64)
        }
    }

    /// What `cmpl` + the predicate's jcc/setcc computes.
    fn wide_cmp(op: IrCmpOp, a: u32, b: u32) -> bool {
        if is_signed(op) {
            holds(op, a as i32 as i64, b as i32 as i64)
        } else {
            holds(op, a as i64, b as i64)
        }
    }

    /// Raw source patterns: every 8-bit pattern exhaustively for 8-bit
    /// images; for 16-bit images every boundary plus a coprime stride.
    fn patterns(img: NarrowImage) -> Vec<u32> {
        if img.bits == 8 {
            return (0..256).collect();
        }
        let mut v: Vec<u32> = (0..65536).step_by(251).collect();
        v.extend([
            0, 1, 0x7f, 0x80, 0xff, 0x100, 0x7fff, 0x8000, 0x8001, 0xff7f, 0xff80, 0xfffe, 0xffff,
        ]);
        v
    }

    #[test]
    fn wide_compare_is_exact_for_every_proven_register_pair() {
        let mut proven = 0usize;
        let mut checks = 0usize;
        for ty in TYS {
            for op in OPS {
                for li in images() {
                    for ri in images() {
                        if wide_cmp_kind(op, ty, Some(li), Some(ri), false).is_none() {
                            continue;
                        }
                        proven += 1;
                        let (pl, pr) = (patterns(li), patterns(ri));
                        for &x in &pl {
                            for &y in &pr {
                                let (a, b) = (image(x, li), image(y, ri));
                                assert_eq!(
                                    wide_cmp(op, a, b),
                                    narrow_cmp(op, ty, a, b),
                                    "{op:?} {ty:?} {li:?}/{ri:?} x={x:#x} y={y:#x}"
                                );
                                checks += 1;
                            }
                        }
                    }
                }
            }
        }
        // 4 types × 10 predicates × 16 image pairs = 640 combinations.
        assert!(proven >= 100, "proven {proven}");
        assert!(checks > 10_000_000, "checks {checks}");
    }

    #[test]
    fn wide_compare_is_exact_for_every_proven_immediate() {
        let imms: Vec<i64> = [
            0i64,
            1,
            -1,
            0x7f,
            0x80,
            -0x80,
            -0x81,
            0xff,
            0x100,
            0x17f,
            300,
            0x7fff,
            0x8000,
            -0x8000,
            -0x8001,
            0xffff,
            0x1_0000,
            0x1_00ff,
            -0x1_0000,
            i32::MAX as i64,
            i32::MIN as i64,
            u32::MAX as i64,
        ]
        .into_iter()
        .chain((-300..300).step_by(7))
        .collect();
        let mut proven = 0usize;
        for ty in TYS {
            for op in OPS {
                for li in images() {
                    let Some(signed) = wide_cmp_kind(op, ty, Some(li), None, true) else {
                        continue;
                    };
                    proven += 1;
                    for &x in &patterns(li) {
                        let a = image(x, li);
                        for &imm in &imms {
                            let wimm = wide_cmp_imm(imm, ty, signed);
                            assert!(
                                i32::try_from(wimm).is_ok(),
                                "imm {wimm} must print as imm32"
                            );
                            assert_eq!(
                                wide_cmp(op, a, wimm as u32),
                                narrow_cmp(op, ty, a, imm as u32),
                                "{op:?} {ty:?} {li:?} x={x:#x} imm={imm}"
                            );
                        }
                    }
                }
            }
        }
        assert!(proven >= 30, "proven {proven}");
    }

    #[test]
    fn wide_compare_proof_obligations() {
        let z8 = NarrowImage {
            bits: 8,
            signed: false,
        };
        let s8 = NarrowImage {
            bits: 8,
            signed: true,
        };
        let z16 = NarrowImage {
            bits: 16,
            signed: false,
        };
        let k = |op, ty, l, r: Option<NarrowImage>| wide_cmp_kind(op, ty, l, r, r.is_none());
        // Proven shapes (the csv/expat `movzbl` byte-in-%edi compares).
        assert_eq!(k(IrCmpOp::Eq, IrType::U8, Some(z8), None), Some(false));
        assert_eq!(k(IrCmpOp::Ult, IrType::U8, Some(z8), None), Some(false));
        assert_eq!(k(IrCmpOp::Slt, IrType::I8, Some(s8), None), Some(true));
        assert_eq!(k(IrCmpOp::Eq, IrType::U8, Some(s8), Some(s8)), Some(true));
        // A zero extension from FEWER bits is also a sign extension.
        assert_eq!(k(IrCmpOp::Slt, IrType::I16, Some(z8), None), Some(true));
        // Refusals: kind mismatch with the predicate, mixed kinds, an image
        // wider than the compare, unknown images, non-narrow compares.
        assert_eq!(k(IrCmpOp::Slt, IrType::I8, Some(z8), None), None);
        assert_eq!(k(IrCmpOp::Ult, IrType::U8, Some(s8), None), None);
        assert_eq!(k(IrCmpOp::Eq, IrType::U8, Some(z8), Some(s8)), None);
        assert_eq!(k(IrCmpOp::Eq, IrType::U8, Some(z16), None), None);
        assert_eq!(k(IrCmpOp::Eq, IrType::U8, None, None), None);
        assert_eq!(
            wide_cmp_kind(IrCmpOp::Eq, IrType::U8, Some(z8), None, false),
            None
        );
        assert_eq!(k(IrCmpOp::Eq, IrType::I32, Some(z8), None), None);
    }

    fn load(dest: u32, ty: IrType) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(0),
            ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }
    fn copy(dest: u32, src: u32) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Value(Value(src)),
        }
    }
    fn phi(dest: u32, ins: Vec<(Operand, u32)>) -> Instruction {
        Instruction::Phi {
            dest: Value(dest),
            ty: IrType::U8,
            incoming: ins.into_iter().map(|(o, b)| (o, BlockId(b))).collect(),
        }
    }
    fn v(n: u32) -> Operand {
        Operand::Value(Value(n))
    }
    fn blk(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }

    #[test]
    fn narrow_images_seed_at_loads_and_flow_only_through_copies_and_phis() {
        let mut f = IrFunction::new("t".to_string(), IrType::I32, Vec::new(), false);
        f.blocks = vec![
            blk(
                0,
                vec![
                    load(1, IrType::U8),
                    load(2, IrType::I8),
                    copy(3, 1),
                    Instruction::BinOp {
                        dest: Value(4),
                        op: IrBinOp::Add,
                        lhs: v(1),
                        rhs: v(1),
                        ty: IrType::U8,
                    },
                    Instruction::Copy {
                        dest: Value(5),
                        src: Operand::Const(IrConst::I8(-1)),
                    },
                    load(6, IrType::I32),
                    load(7, IrType::U16),
                    load(8, IrType::I16),
                    copy(9, 4),
                    load(40, IrType::U8),
                    load(40, IrType::U8),
                ],
                Terminator::Branch(BlockId(1)),
            ),
            blk(
                1,
                vec![
                    // Loop-carried: phi(load, copy(phi)) keeps the image.
                    phi(10, vec![(v(1), 0), (v(12), 1)]),
                    // Mixed zero/sign sources: no image.
                    phi(11, vec![(v(1), 0), (v(2), 1)]),
                    // A constant incoming: no image.
                    phi(13, vec![(v(1), 0), (Operand::Const(IrConst::I8(3)), 1)]),
                    // A pure copy/phi cycle with no producer: dropped.
                    phi(30, vec![(v(31), 0), (v(31), 1)]),
                    copy(31, 30),
                    copy(12, 10),
                    // A loop-carried ALU value poisons the phi.
                    phi(14, vec![(v(1), 0), (v(15), 1)]),
                    Instruction::BinOp {
                        dest: Value(15),
                        op: IrBinOp::Add,
                        lhs: v(14),
                        rhs: v(1),
                        ty: IrType::U8,
                    },
                ],
                Terminator::CondBranch {
                    cond: v(10),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            blk(2, vec![phi(20, vec![(v(4), 1)])], Terminator::Return(None)),
        ];
        f.next_value_id = 41;
        let m = compute_narrow_images(&f);
        let z8 = Some(NarrowImage {
            bits: 8,
            signed: false,
        });
        let s8 = Some(NarrowImage {
            bits: 8,
            signed: true,
        });
        let z16 = Some(NarrowImage {
            bits: 16,
            signed: false,
        });
        let s16 = Some(NarrowImage {
            bits: 16,
            signed: true,
        });
        let got = |n: u32| m.get(&n).copied();
        assert_eq!(got(1), z8);
        assert_eq!(got(2), s8);
        assert_eq!(got(3), z8);
        assert_eq!(got(7), z16);
        assert_eq!(got(8), s16);
        assert_eq!(got(10), z8);
        assert_eq!(got(12), z8);
        for none in [4, 5, 6, 9, 11, 13, 14, 15, 20, 30, 31, 40] {
            assert_eq!(got(none), None, "v{none}");
        }
    }
}
