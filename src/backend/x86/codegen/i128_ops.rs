//! X86Codegen: i128 arithmetic and comparison operations.

use super::emit::X86Codegen;
use crate::backend::generation::is_i128_type;
use crate::backend::state::StackSlot;
use crate::backend::traits::get_const_i128_shift_amount;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::IrType;
use crate::ir::reexports::{Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Value};

/// What the defining instruction of a 128-bit value proves about its high
/// 64 bits.  The absence of a fact is `None` at every use site; there is no
/// "unknown" variant to forget to handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum I128High {
    /// The high half is zero: the value is a zero-extended 64-bit quantity.
    Zero,
    /// The high half is the sign fill of the low half: the value is a
    /// sign-extended 64-bit quantity.
    Sign,
}

/// `X86Codegen::i128_high_facts`: the function the facts were derived from,
/// and the fact per value id (see [`build_i128_high_facts`]).
pub(super) type I128HighFacts = (*const IrFunction, FxHashMap<u32, I128High>);

/// Kill switch for the narrow 128-bit multiplies:
/// `CCC_DISABLE_PASSES=i128_narrow_mul` makes every 128-bit multiply take
/// the generic three-product path again, so a suspected miscompile can be
/// bisected to (or away from) the high-half facts.  A token of the existing
/// pass list rather than a variable of its own, like the allocator's
/// `ipa_ra`.  Cached: the environment never changes mid-compile.
pub(super) fn i128_narrow_mul_allowed() -> bool {
    static OPT_OUT: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        crate::passes::pass_disabled(
            std::env::var("CCC_DISABLE_PASSES").unwrap_or_default(),
            "i128_narrow_mul",
        )
    });
    !*OPT_OUT
}

/// The high-half fact `inst` establishes for the 128-bit value it defines.
///
/// Each arm mirrors the code the emitter produces for that instruction, and
/// the fact is a property of that code -- not of the C type:
///
/// * a widening integer cast stores the 64-bit-extended source in the low
///   half and `xorl %edx, %edx` (unsigned source) or `cqto` (signed source)
///   in the high half (`traits::emit_cast_default`);
/// * a constant right shift by at least 64 moves the old high half down and
///   refills with zero (`LShr`) or with the sign (`AShr`)
///   (`emit_i128_lshr_const_impl` / `emit_i128_ashr_const_impl`).  The
///   amount goes through the same `get_const_i128_shift_amount` the emitter
///   uses to pick the constant form, so an out-of-range amount -- which
///   takes the variable-shift sequence and proves nothing -- has no fact;
/// * an `And` with a full-width constant whose high half is zero clears the
///   high half (`andq %rsi, %rdx` against a zero `%rsi`).  A narrower
///   constant operand is deliberately not accepted: how the pair loaders
///   extend it is an emitter detail no fact should rest on.
///
/// Float and pointer sources, 128-bit sources and every other producer of a
/// 128-bit value (loads, calls, arithmetic) carry no fact.
pub(super) fn i128_high_of_def(inst: &Instruction) -> Option<I128High> {
    match inst {
        Instruction::Cast { from_ty, to_ty, .. } if is_i128_type(*to_ty) => match from_ty {
            IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64 => Some(I128High::Zero),
            IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64 => Some(I128High::Sign),
            _ => None,
        },
        Instruction::BinOp {
            op: op @ (IrBinOp::LShr | IrBinOp::AShr),
            rhs,
            ty,
            ..
        } if is_i128_type(*ty) => match get_const_i128_shift_amount(rhs) {
            Some(amount) if amount >= 64 => Some(if *op == IrBinOp::LShr {
                I128High::Zero
            } else {
                I128High::Sign
            }),
            _ => None,
        },
        Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
            ..
        } if is_i128_type(*ty) => {
            let clears_high = |operand: &Operand| matches!(operand, Operand::Const(IrConst::I128(mask)) if (*mask as u128) >> 64 == 0);
            (clears_high(lhs) || clears_high(rhs)).then_some(I128High::Zero)
        }
        _ => None,
    }
}

/// High-half facts of `func`: value id -> fact, for exactly the values with
/// ONE definition in the whole function, that definition being one
/// [`i128_high_of_def`] recognises.
///
/// The single-definition rule is the soundness condition.  After phi
/// elimination and web coalescing a value id can be written by several
/// instructions (one per predecessor, and not all of them `Copy`), and a
/// fact read off one of them says nothing about the others.  Definitions
/// are therefore counted with the exhaustive `Instruction::dest` -- every
/// value-producing kind, calls and intrinsics included -- rather than with
/// the emitter's def matcher, whose list covers only the re-derivable
/// shapes and would report a `Cast` as unique next to a `Call` writing the
/// same id.
pub(super) fn build_i128_high_facts(func: &IrFunction) -> FxHashMap<u32, I128High> {
    // (fact, definitions seen) per candidate.  Pass 1 nominates every value
    // some instruction would give a fact; pass 2 counts ALL definitions of
    // the nominees, whatever their kind.
    let mut candidates: FxHashMap<u32, (I128High, u32)> = FxHashMap::default();
    for inst in func.blocks.iter().flat_map(|block| &block.instructions) {
        if let (Some(dest), Some(fact)) = (inst.dest(), i128_high_of_def(inst)) {
            candidates.entry(dest.0).or_insert((fact, 0));
        }
    }
    if candidates.is_empty() {
        return FxHashMap::default();
    }
    for inst in func.blocks.iter().flat_map(|block| &block.instructions) {
        if let Some(dest) = inst.dest() {
            if let Some((_, definitions)) = candidates.get_mut(&dest.0) {
                *definitions += 1;
            }
        }
    }
    candidates
        .into_iter()
        .filter(|&(_, (_, definitions))| definitions == 1)
        .map(|(value, (fact, _))| (value, fact))
        .collect()
}

/// A 128-bit multiply narrower than the generic three-product sequence,
/// chosen from the operands' high-half facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum I128MulPlan {
    /// Both operands zero-extended: one widening unsigned multiply.
    Unsigned64,
    /// Both operands sign-extended: one widening signed multiply.
    Signed64,
    /// Right high half zero: the `lhs.lo * rhs.hi` cross term is gone.
    RightHighZero,
    /// Left high half zero: the `lhs.hi * rhs.lo` cross term is gone.
    LeftHighZero,
}

impl I128MulPlan {
    /// The plan for a multiply whose operands carry these facts, or `None`
    /// when only the generic product is correct.
    ///
    /// A sign-extended operand next to anything but another sign-extended one
    /// is treated as fully general: its high half is all ones for a negative
    /// value, so its cross term is real.  A zero-extended and a sign-extended
    /// operand therefore take the one-zero-half plan, never a widening one.
    pub(super) fn select(lhs: Option<I128High>, rhs: Option<I128High>) -> Option<Self> {
        match (lhs, rhs) {
            (Some(I128High::Zero), Some(I128High::Zero)) => Some(Self::Unsigned64),
            (Some(I128High::Sign), Some(I128High::Sign)) => Some(Self::Signed64),
            (_, Some(I128High::Zero)) => Some(Self::RightHighZero),
            (Some(I128High::Zero), _) => Some(Self::LeftHighZero),
            _ => None,
        }
    }

    /// The instructions after operand staging; the product is left in
    /// `%rdx:%rax`.
    ///
    /// Staging contract (`X86Codegen::emit_i128_mul_planned`):
    ///
    /// * `Unsigned64` / `Signed64`: `%rax` = lhs.lo, `%rcx` = rhs.lo.  For
    ///   sign-extended 64-bit values the signed 128-bit product is exact, and
    ///   its low 128 bits are what the 128-bit multiply defines.
    /// * `RightHighZero`: `%rax:%rdx` = lhs, `%rcx` = rhs.lo.
    ///   `hi = high64(lhs.lo * rhs.lo) + lhs.hi * rhs.lo`.
    /// * `LeftHighZero`: `%rax` = lhs.lo, `%rcx:%rsi` = rhs.
    ///   `hi = high64(lhs.lo * rhs.lo) + lhs.lo * rhs.hi`.
    pub(super) fn body(self) -> &'static [&'static str] {
        match self {
            Self::Unsigned64 => &["    mulq %rcx"],
            Self::Signed64 => &["    imulq %rcx"],
            Self::RightHighZero => &[
                "    imulq %rcx, %rdx",
                "    movq %rdx, %rsi",
                "    mulq %rcx",
                "    addq %rsi, %rdx",
            ],
            Self::LeftHighZero => &[
                "    imulq %rax, %rsi",
                "    mulq %rcx",
                "    addq %rsi, %rdx",
            ],
        }
    }
}

impl X86Codegen {
    /// The high-half fact of value `v` in the function being emitted.
    ///
    /// The facts are derived on the first 128-bit multiply of the function
    /// that asks (functions without one pay nothing) and remember the function
    /// they describe: were `current_func` ever re-pointed without the
    /// prologue's reset, the answer is "no fact" -- the generic product --
    /// rather than a fact about another function's value of the same id.
    fn i128_high_fact(&self, v: u32) -> Option<I128High> {
        let func_ptr = self.current_func?;
        let func = self.current_function()?;
        let (built_for, facts) = self
            .i128_high_facts
            .get_or_init(|| (func_ptr, build_i128_high_facts(func)));
        if !std::ptr::eq(*built_for, func_ptr) {
            return None;
        }
        facts.get(&v).copied()
    }

    pub(super) fn emit_i128_prep_binop_impl(&mut self, lhs: &Operand, rhs: &Operand) {
        // Load RHS into %rcx (low) / %rsi (high) FIRST — this only touches
        // rcx/rsi, which no i128 operand can depend on (i128 values live in
        // stack slots or constants). Then load LHS into rax:rdx.
        //
        // CORRECTNESS: the previous implementation saved the LHS with
        // `pushq %rdx; pushq %rax` and restored it with pops. Each push/pop
        // shifts RSP by 8, so every %rsp-relative stack slot emitted inside
        // the window carries a +16 offset. The text-based peephole optimizer
        // does not model this shift: its dead-store and store-forwarding
        // passes compare slot offsets verbatim, so a store before the push
        // window and a load inside it appear to reference different slots.
        // That made the dead-store pass delete LIVE stores in i128
        // pack/unpack sequences (struct-return via i128 packing), producing
        // garbage (e.g. gzip's mtime struct assignment). Avoiding the
        // push/pop removes the shift at the root — and is faster (no
        // stack round-trip).
        self.load_i128_to_rcx_rsi(rhs);
        self.operand_to_rax_rdx(lhs);
        self.state.reg_cache.invalidate_all();
    }

    /// Load an i128 operand into %rcx (low) / %rsi (high) without touching
    /// rax/rdx. The operand is a constant or a stack slot (i128 values are
    /// never register-allocated); constants materialize directly, values are
    /// loaded from their 16-byte slot.
    fn load_i128_to_rcx_rsi(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => {
                match c {
                    IrConst::I128(v) => {
                        let low = *v as u64 as i64;
                        let high = (*v >> 64) as u64 as i64;
                        if low == 0 {
                            self.state.emit("    xorl %ecx, %ecx");
                        } else if low >= i32::MIN as i64 && low <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", low, "rcx");
                        } else {
                            self.state.out.emit_instr_imm_reg("    movabsq", low, "rcx");
                        }
                        if high == 0 {
                            self.state.emit("    xorl %esi, %esi");
                        } else if high >= i32::MIN as i64 && high <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", high, "rsi");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", high, "rsi");
                        }
                    }
                    IrConst::Zero => {
                        self.state.emit("    xorl %ecx, %ecx");
                        self.state.emit("    xorl %esi, %esi");
                    }
                    _ => {
                        // Smaller constant: load into rax (clobbering rax is
                        // fine here — the LHS has not been loaded yet), then
                        // zero-extend the pair into rcx:rsi.
                        self.operand_to_rax(op);
                        self.state.emit("    xorl %edx, %edx");
                        self.state.out.emit_instr_reg_reg("    movq", "rax", "rcx");
                        self.state.out.emit_instr_reg_reg("    movq", "rdx", "rsi");
                    }
                }
            }
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
                    self.state
                        .out
                        .emit_instr_rbp_reg("    movq", slot.0 + 8, "rsi");
                } else {
                    // No slot (should not happen for i128 values): fall back
                    // to the accumulator pair, then copy into rcx:rsi.
                    self.operand_to_rax_rdx(op);
                    self.state.out.emit_instr_reg_reg("    movq", "rax", "rcx");
                    self.state.out.emit_instr_reg_reg("    movq", "rdx", "rsi");
                }
            }
        }
    }

    pub(super) fn emit_i128_add_impl(&mut self) {
        self.state.emit("    addq %rcx, %rax");
        self.state.emit("    adcq %rsi, %rdx");
    }

    pub(super) fn emit_i128_sub_impl(&mut self) {
        self.state.emit("    subq %rcx, %rax");
        self.state.emit("    sbbq %rsi, %rdx");
    }

    /// Generic 128x128 multiply, low 128 bits of the product.
    ///
    /// Entry (`emit_i128_prep_binop_impl`): `%rax` = a.lo, `%rdx` = a.hi,
    /// `%rcx` = b.lo, `%rsi` = b.hi.  Result in `%rdx:%rax`.  Clobbers
    /// `%rsi` and `%rdi`; `%rcx` survives.
    ///
    /// ```text
    /// lo = low64 (a.lo * b.lo)
    /// hi = high64(a.lo * b.lo) + a.lo * b.hi + a.hi * b.lo        (mod 2^64)
    /// ```
    ///
    /// Six register-only instructions.  Both cross products are formed in
    /// place before the widening `mulq` overwrites the pair, so the single
    /// move is the one that parks `a.hi * b.lo` out of `%rdx`.
    ///
    /// The order of the two final adds is the latency contract.  The term
    /// added LAST reaches the result through one `addq`; the other one needs
    /// two.  `a.hi * b.lo` goes last because the left operand is the
    /// loop-carried one in the accumulation shape (`acc *= x`): the recurrence
    /// through `acc.hi` is then `imulq` + `addq` = 4 cycles on every core since
    /// Haswell/Zen, against 5 when the two cross terms are summed first.
    /// Measured on the slot-reloading shape this backend emits (Ice Lake-SP,
    /// minimum ns per multiply): 0.972 against 1.226 for the historical
    /// thirteen-instruction form and 1.219 for the eight-instruction
    /// copy-then-multiply form that keeps the operands in fresh registers
    /// (`engineering/REDTEAM-2026-11-11-I128-TYPEOF.md` has the table and the
    /// llvm-mca model for Raptor Lake).
    ///
    /// The historical form saved the left operand with a `pushq`/`popq` pair
    /// used purely as a register move.  That cost a store-to-load round trip
    /// on the critical path, moved the stack pointer inside a sequence the
    /// text peephole has to model, and clobbered `%rcx`, `%r8` and `%r9` on
    /// top of the registers used here.  The register set of this sequence is a
    /// strict subset of the old one, so the allocator's wide-instruction
    /// model (`regalloc::collect_x64_wide_hazard_points`) covers it unchanged.
    pub(super) fn emit_i128_mul_impl(&mut self) {
        self.state.emit("    imulq %rcx, %rdx");
        self.state.emit("    imulq %rax, %rsi");
        self.state.emit("    movq %rdx, %rdi");
        self.state.emit("    mulq %rcx");
        self.state.emit("    addq %rsi, %rdx");
        self.state.emit("    addq %rdi, %rdx");
    }

    /// The high-half fact, and the slot whose low half holds the value, of a
    /// 128-bit multiply operand -- `None` when nothing is provable.
    ///
    /// Only a genuine slot-homed 128-bit value qualifies: the fact describes
    /// the two halves its defining instruction stored, and the multiply reads
    /// the low half back from that slot.  It deliberately does NOT read the
    /// 64-bit source of a widening cast: that value's live range may end at
    /// the cast, so its register can already hold something else here.
    fn i128_narrow_operand(&self, op: &Operand) -> Option<(I128High, StackSlot)> {
        let Operand::Value(v) = op else {
            return None;
        };
        if !self.state.is_i128_value(v.0) || self.state.is_alloca(v.0) {
            return None;
        }
        let slot = self.state.get_slot(v.0)?;
        let high = self.i128_high_fact(v.0)?;
        Some((high, slot))
    }

    /// Multiply two 128-bit operands whose high halves are statically known,
    /// leaving the product in `%rdx:%rax`.  Returns false -- having emitted
    /// nothing -- when no narrower form than the generic product applies.
    ///
    /// `(unsigned __int128)a * b` with 64-bit `a` and `b` is the dominant
    /// 128-bit multiply in real code (bignum limbs, modular reduction, hash
    /// mixing).  Both operands are zero-extended, so two of the generic
    /// product's three multiplies have a zero factor and the whole operation
    /// is the single widening `mulq` every other compiler emits.  The signed
    /// twin is one-operand `imulq`; with only one operand narrow, the cross
    /// term through its high half disappears.
    ///
    /// Disable with `CCC_DISABLE_PASSES=i128_narrow_mul`.
    pub(super) fn emit_i128_mul_narrow_impl(&mut self, lhs: &Operand, rhs: &Operand) -> bool {
        if !i128_narrow_mul_allowed() {
            return false;
        }
        let left = self.i128_narrow_operand(lhs);
        let right = self.i128_narrow_operand(rhs);
        let Some(plan) = I128MulPlan::select(left.map(|f| f.0), right.map(|f| f.0)) else {
            return false;
        };
        self.emit_i128_mul_planned(plan, lhs, rhs, left.map(|f| f.1), right.map(|f| f.1))
    }

    /// Stage the operands `plan` needs and emit its body.  `left_low` /
    /// `right_low` are the slots of the operands the plan reads as a single
    /// low half; a plan whose narrow operand has no slot is refused before
    /// anything is emitted.
    fn emit_i128_mul_planned(
        &mut self,
        plan: I128MulPlan,
        lhs: &Operand,
        rhs: &Operand,
        left_low: Option<StackSlot>,
        right_low: Option<StackSlot>,
    ) -> bool {
        // The right operand is staged first, as in the generic prep: loading
        // it may use %rax/%rdx as scratch, which the left operand then owns.
        match plan {
            I128MulPlan::Unsigned64 | I128MulPlan::Signed64 => {
                let (Some(left), Some(right)) = (left_low, right_low) else {
                    return false;
                };
                self.begin_raw_i128_sequence();
                self.state
                    .out
                    .emit_instr_rbp_reg("    movq", right.0, "rcx");
                self.state.out.emit_instr_rbp_reg("    movq", left.0, "rax");
            }
            I128MulPlan::RightHighZero => {
                let Some(right) = right_low else {
                    return false;
                };
                self.begin_raw_i128_sequence();
                self.state
                    .out
                    .emit_instr_rbp_reg("    movq", right.0, "rcx");
                self.operand_to_rax_rdx(lhs);
            }
            I128MulPlan::LeftHighZero => {
                let Some(left) = left_low else {
                    return false;
                };
                self.begin_raw_i128_sequence();
                self.load_i128_to_rcx_rsi(rhs);
                self.state.out.emit_instr_rbp_reg("    movq", left.0, "rax");
            }
        }
        for line in plan.body() {
            self.state.emit(line);
        }
        true
    }

    /// State the accumulator-cache invariant for a raw-emit i128 sequence, and
    /// start the sequence from a known-empty cache.
    ///
    /// A 128-bit value lives in the `%rax:%rdx` PAIR, and `%rax` alone is only
    /// the low half -- not a valid scalar IR value.  Every sequence opened with
    /// this call therefore performs raw emits that clobber `%rax`/`%rdx` (and
    /// `%rcx` or `%rsi` as scratch) and leaves the cache alone until the value
    /// is committed by `store_rax_rdx_to`, which invalidates on the way out.
    ///
    /// THE INVARIANT: between this call and that final invalidation the
    /// sequence may perform exactly ONE cache-consulting load -- the
    /// `operand_to_rax_rdx` below, which runs BEFORE the first clobber and is
    /// therefore sound.  Any further cache-consulting load (an `operand_to_rax`
    /// on a small constant, say) placed after a clobber would read a cache that
    /// still claims `%rax` for a value the clobber destroyed, and would hand
    /// back the wrong register -- silently, since the resulting code still
    /// assembles.  Keep such a load out of the region, or invalidate again
    /// after it.
    ///
    /// The entry invalidation is cheap insurance rather than a fix for a
    /// present bug: the pair path never consults the scalar cache (verified by
    /// inspection -- `operand_to_rax_rdx` contains no `reg_cache` reference at
    /// all) and the exit invalidation closes the window.  It means the region
    /// cannot inherit a stale entry from its CALLER, which is the half of the
    /// hazard the exit invalidation cannot cover.
    #[inline]
    fn begin_raw_i128_sequence(&mut self) {
        self.state.reg_cache.invalidate_all();
    }

    /// Constant-multiplier multiply: `(lhs.lo:lhs.hi) * (lo:hi)`.
    ///
    /// The generic sequence (`emit_i128_mul_impl`) is a full 128x128
    /// schoolbook product: three multiplies, with both operands materialised
    /// in registers first.  With the multiplier's halves known at compile
    /// time the cross terms that involve a zero half disappear, and the
    /// constant never has to be loaded into the accumulator pair.
    ///
    /// This is the hot shape for multiply-by-magic code (hash mixing, fixed
    /// point scaling, hand-written reciprocal division), whose 128-bit
    /// constant usually has a zero high half: the sequence then collapses to
    /// five instructions -- and to the bare widening `mulq` when the left
    /// operand is itself a zero-extended 64-bit value -- matching what GCC,
    /// Clang and ICX emit.
    pub(super) fn emit_i128_mul_const_impl(
        &mut self,
        lhs: &Operand,
        _rhs: &Operand,
        lo: u64,
        hi: u64,
    ) {
        let left_zero_high = match self.i128_narrow_operand(lhs) {
            Some((I128High::Zero, slot)) if i128_narrow_mul_allowed() => Some(slot),
            _ => None,
        };
        self.emit_i128_mul_const_with(lhs, lo, hi, left_zero_high);
    }

    /// `emit_i128_mul_const_impl` with the left operand's fact resolved:
    /// `left_zero_high` is the slot of a left operand whose high half is
    /// provably zero, read as its low half alone.
    fn emit_i128_mul_const_with(
        &mut self,
        lhs: &Operand,
        lo: u64,
        hi: u64,
        left_zero_high: Option<StackSlot>,
    ) {
        self.begin_raw_i128_sequence();
        // `mulq` overwrites both accumulator halves, so park the LHS halves
        // that the cross terms still need first.  lhs.hi is only needed when
        // it can be non-zero, lhs.lo only when the multiplier has a high half.
        if let Some(slot) = left_zero_high {
            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
        } else {
            // LHS into %rax:%rdx (low:high); %rcx/%r8/%rsi are scratch.  The
            // one permitted cache-consulting load, before any clobber.
            self.operand_to_rax_rdx(lhs);
            self.state.emit("    movq %rdx, %rcx");
        }
        if hi != 0 {
            self.state.emit("    movq %rax, %rsi");
        }
        self.emit_movabs_to_r8(lo);
        // Unsigned low product; its high half lands in %rdx.
        self.state.emit("    mulq %r8");
        if left_zero_high.is_none() {
            // Cross term lhs.hi * lo.
            self.state.emit("    imulq %r8, %rcx");
            self.state.emit("    addq %rcx, %rdx");
        }
        if hi != 0 {
            // Second cross term lhs.lo * hi, against the parked lhs.lo.
            self.emit_movabs_to_r8(hi);
            self.state.emit("    imulq %r8, %rsi");
            self.state.emit("    addq %rsi, %rdx");
        }
    }

    /// Load a 64-bit immediate into %r8, using the short form when it fits.
    fn emit_movabs_to_r8(&mut self, v: u64) {
        if v <= 0x7fff_ffff {
            // Fits in a sign-extended imm32, so no REX.W movabs is needed.
            self.state.emit_fmt(format_args!("    movl ${}, %r8d", v));
        } else {
            self.state
                .emit_fmt(format_args!("    movabsq ${}, %r8", v as i64));
        }
    }

    pub(super) fn emit_i128_and_impl(&mut self) {
        self.state.emit("    andq %rcx, %rax");
        self.state.emit("    andq %rsi, %rdx");
    }

    pub(super) fn emit_i128_or_impl(&mut self) {
        self.state.emit("    orq %rcx, %rax");
        self.state.emit("    orq %rsi, %rdx");
    }

    pub(super) fn emit_i128_xor_impl(&mut self) {
        self.state.emit("    xorq %rcx, %rax");
        self.state.emit("    xorq %rsi, %rdx");
    }

    pub(super) fn emit_i128_shl_impl(&mut self) {
        self.state.emit("    shldq %cl, %rax, %rdx");
        self.state.emit("    shlq %cl, %rax");
        self.state.emit("    testb $64, %cl");
        self.state.emit("    je 1f");
        self.state.emit("    movq %rax, %rdx");
        self.state.emit("    xorl %eax, %eax");
        self.state.emit("1:");
    }

    pub(super) fn emit_i128_lshr_impl(&mut self) {
        self.state.emit("    shrdq %cl, %rdx, %rax");
        self.state.emit("    shrq %cl, %rdx");
        self.state.emit("    testb $64, %cl");
        self.state.emit("    je 1f");
        self.state.emit("    movq %rdx, %rax");
        self.state.emit("    xorl %edx, %edx");
        self.state.emit("1:");
    }

    pub(super) fn emit_i128_ashr_impl(&mut self) {
        self.state.emit("    shrdq %cl, %rdx, %rax");
        self.state.emit("    sarq %cl, %rdx");
        self.state.emit("    testb $64, %cl");
        self.state.emit("    je 1f");
        self.state.emit("    movq %rdx, %rax");
        self.state.emit("    sarq $63, %rdx");
        self.state.emit("1:");
    }

    pub(super) fn emit_i128_prep_shift_lhs_impl(&mut self, lhs: &Operand) {
        // Opens the shift sequences, which clobber the pair with raw emits the
        // same way the multiply does; see `begin_raw_i128_sequence`.
        self.begin_raw_i128_sequence();
        self.operand_to_rax_rdx(lhs);
    }

    pub(super) fn emit_i128_shl_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            // no-op
        } else if amount == 64 {
            self.state.emit("    movq %rax, %rdx");
            self.state.emit("    xorl %eax, %eax");
        } else if amount > 64 {
            self.state
                .out
                .emit_instr_imm_reg("    shlq", (amount - 64) as i64, "rax");
            self.state.emit("    movq %rax, %rdx");
            self.state.emit("    xorl %eax, %eax");
        } else {
            self.state
                .emit_fmt(format_args!("    shldq ${}, %rax, %rdx", amount));
            self.state
                .out
                .emit_instr_imm_reg("    shlq", amount as i64, "rax");
        }
    }

    pub(super) fn emit_i128_lshr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            // no-op
        } else if amount == 64 {
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    xorl %edx, %edx");
        } else if amount > 64 {
            self.state
                .out
                .emit_instr_imm_reg("    shrq", (amount - 64) as i64, "rdx");
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    xorl %edx, %edx");
        } else {
            self.state
                .emit_fmt(format_args!("    shrdq ${}, %rdx, %rax", amount));
            self.state
                .out
                .emit_instr_imm_reg("    shrq", amount as i64, "rdx");
        }
    }

    pub(super) fn emit_i128_ashr_const_impl(&mut self, amount: u32) {
        let amount = amount & 127;
        if amount == 0 {
            // no-op
        } else if amount == 64 {
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    sarq $63, %rdx");
        } else if amount > 64 {
            self.state
                .out
                .emit_instr_imm_reg("    sarq", (amount - 64) as i64, "rdx");
            self.state.emit("    movq %rdx, %rax");
            self.state.emit("    sarq $63, %rdx");
        } else {
            self.state
                .emit_fmt(format_args!("    shrdq ${}, %rdx, %rax", amount));
            self.state
                .out
                .emit_instr_imm_reg("    sarq", amount as i64, "rdx");
        }
    }

    pub(super) fn emit_i128_divrem_call_impl(
        &mut self,
        func_name: &str,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        self.operand_to_rax_rdx(rhs);
        self.state.emit("    pushq %rdx");
        self.state.emit("    pushq %rax");
        // In frame-pointer-omitted mode every logical slot is addressed from
        // %rsp. The two saves above lowered the physical stack pointer by 16;
        // without compensating, loading lhs reads two slots early (often zeros
        // or an unrelated i128 half). Treat the temporary push depth exactly
        // like the inline-asm after-push path does.
        let saved_frame_size = self.state.out.rsp_frame_size;
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += 16;
        }
        self.operand_to_rax_rdx(lhs);
        self.state.out.rsp_frame_size = saved_frame_size;
        self.state.emit("    movq %rax, %rdi");
        self.state.emit("    movq %rdx, %rsi");
        self.state.emit("    popq %rdx");
        self.state.emit("    popq %rcx");
        self.state
            .emit_fmt(format_args!("    call {}@PLT", func_name));
    }

    pub(super) fn emit_i128_store_result_impl(&mut self, dest: &Value) {
        self.store_rax_rdx_to(dest);
    }

    pub(super) fn emit_i128_to_float_call_impl(
        &mut self,
        src: &Operand,
        from_signed: bool,
        to_ty: IrType,
    ) {
        self.operand_to_rax_rdx(src);
        self.state.emit("    movq %rax, %rdi");
        self.state.emit("    movq %rdx, %rsi");
        let func_name = match (from_signed, to_ty) {
            (true, IrType::F64) => "__floattidf",
            (true, IrType::F32) => "__floattisf",
            (true, IrType::F128) => "__floattitf",
            (false, IrType::F64) => "__floatuntidf",
            (false, IrType::F32) => "__floatuntisf",
            (false, IrType::F128) => "__floatuntitf",
            _ => panic!("unsupported i128-to-float conversion: {:?}", to_ty),
        };
        self.state
            .emit_fmt(format_args!("    call {}@PLT", func_name));
        self.state.reg_cache.invalidate_all();
        if to_ty == IrType::F128 {
            // _Float128 result stays in %xmm0 (single-XMM SysV ABI).
            return;
        }
        self.emit_scalar_xmm_to_gpr("xmm0", "rax", (to_ty == IrType::F32).then_some("eax"));
    }

    pub(super) fn emit_float_to_i128_call_impl(
        &mut self,
        src: &Operand,
        to_signed: bool,
        from_ty: IrType,
    ) {
        if from_ty == IrType::F128 {
            // _Float128 (binary128): ONE XMM register per SysV psABI. Load the
            // 16-byte value into %xmm0, then call libgcc's __fixtfti /
            // __fixunstfti (i128 result in %rdx:%rax).
            match src {
                Operand::Value(v) => {
                    // LAZY-FLUSH SOUNDNESS (the `vec_arg_mem` / `evex_arg_mem`
                    // discipline). A deferred vector store means this value's
                    // slot has NOT been written yet, so the slot load below
                    // would read uninitialised stack. Commit the store first:
                    // the slot then holds the value and the load stays the
                    // cheapest form (`movdqu`), which is why this commits
                    // rather than taking the register path — an F128 value can
                    // be the deferred result without any claim (banks 0/1 are
                    // unclaimable), so refusing to fold would reload from the
                    // same unwritten slot.
                    //
                    // The value test is required: an unrelated deferred store
                    // ("always flush") would delete the deferral win on every
                    // following conversion.
                    if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                        self.flush_pending_vec_store_impl();
                    }
                    // Registers before slots, as everywhere else: the claim
                    // says where the 16 bytes ARE, and a claim is recorded only
                    // after a real write of `v` into that bank. Read through
                    // `vec_live_xmm` rather than the raw map — the map is
                    // width-polyglot and `movdqa %ymmN, %xmm0` /
                    // `movdqa %zmmN, %xmm0` are illegal mixed-width encodings
                    // that the assembler rejects outright.
                    if let Some(held) = self.state.vec_live_xmm(v.0) {
                        if held != "xmm0" {
                            self.state
                                .emit_fmt(format_args!("    movdqa %{}, %xmm0", held));
                        }
                    } else if let Some(slot) = self.state.get_slot(v.0) {
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movdqu", slot.0, "xmm0");
                    } else {
                        self.operand_to_reg(src, "rax");
                        self.state.emit("    movdqu (%rax), %xmm0");
                    }
                }
                _ => {
                    self.operand_to_reg(src, "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                }
            }
            let func_name = if to_signed {
                "__fixtfti"
            } else {
                "__fixunstfti"
            };
            self.state
                .emit_fmt(format_args!("    call {}@PLT", func_name));
            self.state.reg_cache.invalidate_all();
            return;
        }
        self.operand_to_rax(src);
        if from_ty == IrType::F32 {
            self.state.emit("    movd %eax, %xmm0");
        } else {
            self.state.emit("    movq %rax, %xmm0");
        }
        let func_name = match (to_signed, from_ty) {
            (true, IrType::F64) => "__fixdfti",
            (true, IrType::F32) => "__fixsfti",
            (false, IrType::F64) => "__fixunsdfti",
            (false, IrType::F32) => "__fixunssfti",
            _ => panic!("unsupported float-to-i128 conversion: {:?}", from_ty),
        };
        self.state
            .emit_fmt(format_args!("    call {}@PLT", func_name));
        self.state.reg_cache.invalidate_all();
    }

    // ---- i128 cmp primitives ----

    pub(super) fn emit_i128_cmp_eq_impl(&mut self, is_ne: bool) {
        self.state.emit("    xorq %rcx, %rax");
        self.state.emit("    xorq %rsi, %rdx");
        self.state.emit("    orq %rdx, %rax");
        if is_ne {
            self.state.emit("    setne %al");
        } else {
            self.state.emit("    sete %al");
        }
        self.state.emit("    movzbl %al, %eax");
    }

    pub(super) fn emit_i128_cmp_ordered_impl(&mut self, op: IrCmpOp) {
        self.state.emit("    cmpq %rsi, %rdx");
        let set_hi = match op {
            IrCmpOp::Slt | IrCmpOp::Sle => "setl",
            IrCmpOp::Sgt | IrCmpOp::Sge => "setg",
            IrCmpOp::Ult | IrCmpOp::Ule => "setb",
            IrCmpOp::Ugt | IrCmpOp::Uge => "seta",
            _ => unreachable!("i128 ordered cmp got equality op: {:?}", op),
        };
        self.state.emit_fmt(format_args!("    {} %r8b", set_hi));
        self.state.emit("    jne 1f");
        self.state.emit("    cmpq %rcx, %rax");
        let set_lo = match op {
            IrCmpOp::Slt | IrCmpOp::Ult => "setb",
            IrCmpOp::Sle | IrCmpOp::Ule => "setbe",
            IrCmpOp::Sgt | IrCmpOp::Ugt => "seta",
            IrCmpOp::Sge | IrCmpOp::Uge => "setae",
            _ => unreachable!("i128 ordered cmp (low word) got equality op: {:?}", op),
        };
        self.state.emit_fmt(format_args!("    {} %r8b", set_lo));
        self.state.emit("1:");
        self.state.emit("    movzbl %r8b, %eax");
    }

    pub(super) fn emit_i128_cmp_store_result_impl(&mut self, dest: &Value) {
        self.store_rax_to(dest);
    }

    // ---- i128 load/store acc pair ----

    pub(super) fn emit_load_acc_pair_impl(&mut self, op: &Operand) {
        self.operand_to_rax_rdx(op);
    }

    pub(super) fn emit_store_acc_pair_impl(&mut self, dest: &Value) {
        self.store_rax_rdx_to(dest);
    }

    pub(super) fn emit_sign_extend_acc_high_impl(&mut self) {
        self.state.emit("    cqto"); // sign-extend rax into rdx:rax
    }

    pub(super) fn emit_zero_acc_high_impl(&mut self) {
        self.state.emit("    xorl %edx, %edx");
    }

    pub(super) fn emit_store_pair_to_slot_impl(&mut self, slot: StackSlot) {
        self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
        self.state
            .out
            .emit_instr_reg_rbp("    movq", "rdx", slot.0 + 8);
    }

    pub(super) fn emit_load_pair_from_slot_impl(&mut self, slot: StackSlot) {
        self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
        self.state
            .out
            .emit_instr_rbp_reg("    movq", slot.0 + 8, "rdx");
    }

    pub(super) fn emit_save_acc_pair_impl(&mut self) {
        self.state.emit("    movq %rax, %rsi");
        self.state.emit("    movq %rdx, %rdi");
    }

    pub(super) fn emit_store_pair_indirect_impl(&mut self) {
        // pair saved to rsi:rdi by emit_save_acc_pair, ptr in rcx via emit_load_ptr_from_slot
        self.state.emit("    movq %rsi, (%rcx)");
        self.state.emit("    movq %rdi, 8(%rcx)");
    }

    pub(super) fn emit_load_pair_indirect_impl(&mut self) {
        // ptr in rcx via emit_load_ptr_from_slot, load pair from it
        self.state.emit("    movq (%rcx), %rax");
        self.state.emit("    movq 8(%rcx), %rdx");
    }

    pub(super) fn emit_i128_neg_impl(&mut self) {
        self.state.emit("    notq %rax");
        self.state.emit("    notq %rdx");
        self.state.emit("    addq $1, %rax");
        self.state.emit("    adcq $0, %rdx");
    }

    pub(super) fn emit_i128_not_impl(&mut self) {
        self.state.emit("    notq %rax");
        self.state.emit("    notq %rdx");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LAW E at the F128 boundary: a DEFERRED vector store means the value's
    /// slot has not been written, so a slot-first load reads uninitialised
    /// stack and `__fixtfti` converts garbage.
    ///
    /// The F128 value here is the deferred one and carries NO claim — that is
    /// the reachable shape, not a contrived one: `vec_claim_live_reg` refuses
    /// the reserved scratch pair, so a value left in `%xmm0` by a 128-bit
    /// producer is described only by `pending_vec_store`. The three asserts
    /// are ordered as the contract: the store must be committed FIRST, into
    /// the value's own slot, and only then may the load read it.
    #[test]
    fn f128_conversion_commits_a_deferred_store_before_reading_the_slot() {
        let mut cg = X86Codegen::new();
        cg.state.value_locations.insert(3, StackSlot(-48));
        cg.state.vector128_values.insert(3);
        cg.state.pending_vec_store = Some((3, "xmm0", false));
        cg.emit_float_to_i128_call_impl(&Operand::Value(Value(3)), true, IrType::F128);

        let out = cg.state.out.buf.clone();
        let store = out
            .find("movdqu %xmm0, -48(%rbp)")
            .or_else(|| out.find("movups %xmm0, -48(%rbp)"))
            .unwrap_or_else(|| {
                panic!("the deferred F128 store must be committed to its slot: {out}")
            });
        let load = out
            .find("movdqu -48(%rbp), %xmm0")
            .or_else(|| out.find("movups -48(%rbp), %xmm0"))
            .unwrap_or_else(|| {
                panic!("the value must be loaded into %xmm0 for the libcall: {out}")
            });
        assert!(
            store < load,
            "the commit must precede the load, or the load reads uninitialised \
             stack: {out}"
        );
        assert!(
            cg.state.pending_vec_store.is_none(),
            "the deferral must be consumed, not merely found: {out}"
        );
        assert!(
            out.contains("call __fixtfti@PLT"),
            "the libcall must still be emitted: {out}"
        );
    }

    /// The commit is VALUE-SPECIFIC: an unrelated deferral must survive, or
    /// the deferred-store optimisation collapses to \"always flush\" and every
    /// conversion after a deferred producer pays a store it does not need.
    #[test]
    fn f128_conversion_leaves_an_unrelated_deferred_store_alone() {
        let mut cg = X86Codegen::new();
        cg.state.value_locations.insert(3, StackSlot(-48));
        cg.state.vector128_values.insert(3);
        cg.state.pending_vec_store = Some((7, "xmm0", false));
        cg.emit_float_to_i128_call_impl(&Operand::Value(Value(3)), true, IrType::F128);
        assert_eq!(
            cg.state.pending_vec_store,
            Some((7, "xmm0", false)),
            "an unrelated value's deferral must not be flushed"
        );
    }

    /// Registers before slots, and the claim must be WIDTH-FILTERED. The map
    /// is polyglot (`%xmmN` / `%ymmN` / `%zmmN`), and this consumer emits
    /// `movdqa %held, %xmm0`, which is an illegal mixed-width encoding for a
    /// `%ymm`/`%zmm` name. A width-mismatched claim therefore falls back to
    /// the (valid, now committed) slot instead of failing to assemble.
    #[test]
    fn f128_conversion_uses_only_width_matched_claims() {
        // A `%xmm` claim wins over the slot, and needs no move when it is
        // already `%xmm0`.
        let mut cg = X86Codegen::new();
        cg.state.value_locations.insert(3, StackSlot(-48));
        cg.state.vector128_values.insert(3);
        cg.state.vec_live_regs.insert(3, "xmm5");
        cg.emit_float_to_i128_call_impl(&Operand::Value(Value(3)), true, IrType::F128);
        let out = cg.state.out.buf.clone();
        assert!(
            out.contains("movdqa %xmm5, %xmm0"),
            "a width-matched claim must be moved in: {out}"
        );
        assert!(
            !out.contains("movdqu -48(%rbp)"),
            "and the slot must not be read as well: {out}"
        );

        // A claim at the wrong width is reported absent: the slot load runs.
        for wrong in ["ymm5", "zmm5"] {
            let mut cg = X86Codegen::new();
            cg.state.value_locations.insert(3, StackSlot(-48));
            cg.state.vector128_values.insert(3);
            cg.state.vec_live_regs.insert(3, wrong);
            cg.emit_float_to_i128_call_impl(&Operand::Value(Value(3)), true, IrType::F128);
            let out = cg.state.out.buf.clone();
            assert!(
                !out.contains("movdqa %"),
                "a {wrong} claim must not be spliced into an xmm operand: {out}"
            );
            assert!(
                out.contains("movdqu -48(%rbp), %xmm0"),
                "the slot load must run instead: {out}"
            );
        }
    }

    // ---- 128-bit multiply: emitter contracts and an executing model ----

    use crate::ir::reexports::{BasicBlock, BlockId, Terminator};
    use std::collections::HashMap;

    /// Executes the straight-line AT&T subset the 128-bit multiply emitters
    /// produce, so the tests below check what the emitted TEXT computes
    /// rather than a hand-copied arithmetic model of it.
    ///
    /// Reading a register nothing has written is a hard failure: a sequence
    /// that silently depended on a stale scratch register would otherwise
    /// pass on whatever the previous case left behind.
    #[derive(Default)]
    struct Machine {
        regs: HashMap<&'static str, u64>,
        slots: HashMap<i64, u64>,
    }

    fn reg64(name: &str) -> &'static str {
        match name {
            "rax" | "eax" => "rax",
            "rdx" | "edx" => "rdx",
            "rcx" | "ecx" => "rcx",
            "rsi" | "esi" => "rsi",
            "rdi" | "edi" => "rdi",
            "r8" | "r8d" => "r8",
            "r9" | "r9d" => "r9",
            other => panic!("unmodelled register %{other}"),
        }
    }

    impl Machine {
        fn with(regs: &[(&str, u64)]) -> Self {
            let mut machine = Machine::default();
            for &(name, value) in regs {
                machine.set(name, value);
            }
            machine
        }

        fn get(&self, name: &str) -> u64 {
            *self
                .regs
                .get(reg64(name))
                .unwrap_or_else(|| panic!("%{name} is read before anything wrote it"))
        }

        fn set(&mut self, name: &str, value: u64) {
            self.regs.insert(reg64(name), value);
        }

        fn pair(&self) -> u128 {
            ((self.get("rdx") as u128) << 64) | self.get("rax") as u128
        }

        fn read(&self, operand: &str) -> u64 {
            if let Some(name) = operand.strip_prefix('%') {
                self.get(name)
            } else if let Some(imm) = operand.strip_prefix('$') {
                imm.parse::<i64>()
                    .unwrap_or_else(|_| panic!("unmodelled immediate {operand}"))
                    as u64
            } else if let Some(offset) = operand.strip_suffix("(%rbp)") {
                let offset: i64 = offset
                    .parse()
                    .unwrap_or_else(|_| panic!("unmodelled memory operand {operand}"));
                *self
                    .slots
                    .get(&offset)
                    .unwrap_or_else(|| panic!("slot {offset} is read before anything wrote it"))
            } else {
                panic!("unmodelled operand {operand}")
            }
        }

        fn run(&mut self, asm: &str) -> &mut Self {
            for line in asm.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let (mnemonic, rest) = line
                    .split_once(' ')
                    .unwrap_or_else(|| panic!("unmodelled instruction: {line}"));
                let operands: Vec<&str> = rest.split(',').map(str::trim).collect();
                let dest = |operand: &str| -> String {
                    operand
                        .strip_prefix('%')
                        .unwrap_or_else(|| panic!("unmodelled destination in: {line}"))
                        .to_string()
                };
                match (mnemonic, operands.as_slice()) {
                    ("movq" | "movabsq", [src, dst]) => {
                        let value = self.read(src);
                        self.set(&dest(dst), value);
                    }
                    ("movl", [src, dst]) => {
                        let value = self.read(src) as u32 as u64;
                        self.set(&dest(dst), value);
                    }
                    ("xorl", [a, b]) if a == b => self.set(&dest(b), 0),
                    ("addq", [src, dst]) => {
                        let value = self.get(&dest(dst)).wrapping_add(self.read(src));
                        self.set(&dest(dst), value);
                    }
                    ("imulq", [src, dst]) => {
                        let value = self.get(&dest(dst)).wrapping_mul(self.read(src));
                        self.set(&dest(dst), value);
                    }
                    ("mulq", [src]) => {
                        let product = self.get("rax") as u128 * self.read(src) as u128;
                        self.set("rax", product as u64);
                        self.set("rdx", (product >> 64) as u64);
                    }
                    ("imulq", [src]) => {
                        let product =
                            (self.get("rax") as i64 as i128) * (self.read(src) as i64 as i128);
                        self.set("rax", product as u64);
                        self.set("rdx", ((product as u128) >> 64) as u64);
                    }
                    _ => panic!("unmodelled instruction: {line}"),
                }
            }
            self
        }
    }

    /// Hard edges first, then a deterministic LCG sweep: `count` operand pairs.
    fn multiply_cases(count: usize) -> Vec<(u128, u128)> {
        const EDGES: [u128; 9] = [
            0,
            1,
            u64::MAX as u128,
            1u128 << 63,
            1u128 << 64,
            (1u128 << 64) - 1 + (1u128 << 127),
            u128::MAX,
            1u128 << 127,
            0x9E37_79B9_7F4A_7C15_C2B2_AE3D_27D4_EB4F,
        ];
        let mut cases = Vec::with_capacity(count);
        for a in EDGES {
            for b in EDGES {
                cases.push((a, b));
            }
        }
        let mut state: u64 = 0x243F_6A88_85A3_08D3;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state
        };
        while cases.len() < count {
            let a = ((next() as u128) << 64) | next() as u128;
            let b = ((next() as u128) << 64) | next() as u128;
            cases.push((a, b));
        }
        cases
    }

    fn lo(v: u128) -> u64 {
        v as u64
    }

    fn hi(v: u128) -> u64 {
        (v >> 64) as u64
    }

    fn zext(v: u128) -> u128 {
        lo(v) as u128
    }

    fn sext(v: u128) -> u128 {
        lo(v) as i64 as i128 as u128
    }

    fn emitted_lines(cg: &X86Codegen) -> Vec<String> {
        cg.state
            .out
            .buf
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// The historical generic multiply, kept verbatim as the negative control.
    const HISTORICAL_MULTIPLY: &str = "    pushq %rdx\n    pushq %rax\n    movq %rcx, %r8\n    \
        movq %rsi, %r9\n    popq %rax\n    popq %rdi\n    movq %rdi, %rcx\n    imulq %r8, %rcx\n    \
        movq %rax, %rsi\n    imulq %r9, %rsi\n    mulq %r8\n    addq %rcx, %rdx\n    addq %rsi, %rdx\n";

    /// What the generic multiply must never contain again: a stack
    /// round-trip, any stack-pointer reference, or a register outside the
    /// set the allocator's wide-instruction model was built for minus the
    /// three the new sequence released.
    fn stack_or_wide_scratch_use(asm: &str) -> Vec<&'static str> {
        ["pushq", "popq", "%rsp", "%r8", "%r9"]
            .into_iter()
            .filter(|needle| asm.contains(needle))
            .collect()
    }

    #[test]
    fn generic_i128_multiply_is_six_register_only_instructions() {
        let mut cg = X86Codegen::new();
        cg.emit_i128_mul_impl();
        assert_eq!(
            emitted_lines(&cg),
            [
                "imulq %rcx, %rdx",
                "imulq %rax, %rsi",
                "movq %rdx, %rdi",
                "mulq %rcx",
                "addq %rsi, %rdx",
                "addq %rdi, %rdx",
            ],
            "the cross term through lhs.hi is added last: that order is the \
             latency contract of the accumulation shape"
        );
        assert_eq!(
            stack_or_wide_scratch_use(&cg.state.out.buf),
            Vec::<&str>::new()
        );
    }

    /// Negative control: the detector above must fire on the sequence it
    /// exists to keep out, or a green result proves nothing.
    #[test]
    fn historical_push_pop_multiply_is_what_the_contract_rejects() {
        assert_eq!(
            stack_or_wide_scratch_use(HISTORICAL_MULTIPLY),
            ["pushq", "popq", "%r8", "%r9"]
        );
    }

    #[test]
    fn generic_i128_multiply_matches_wrapping_mul_and_keeps_rcx() {
        let mut cg = X86Codegen::new();
        cg.emit_i128_mul_impl();
        let asm = cg.state.out.buf.clone();
        let cases = multiply_cases(1000);
        assert_eq!(cases.len(), 1000);
        for (a, b) in cases {
            let mut machine = Machine::with(&[
                ("rax", lo(a)),
                ("rdx", hi(a)),
                ("rcx", lo(b)),
                ("rsi", hi(b)),
            ]);
            machine.run(&asm);
            assert_eq!(machine.pair(), a.wrapping_mul(b), "{a:#x} * {b:#x}");
            assert_eq!(machine.get("rcx"), lo(b), "%rcx must survive");
            let mut written: Vec<&str> = machine.regs.keys().copied().collect();
            written.sort_unstable();
            assert_eq!(
                written,
                ["rax", "rcx", "rdi", "rdx", "rsi"],
                "no register outside the documented set may be touched"
            );
        }
    }

    #[test]
    fn narrow_multiply_plans_follow_the_high_half_facts() {
        use I128High::{Sign, Zero};
        use I128MulPlan::{LeftHighZero, RightHighZero, Signed64, Unsigned64};
        let table = [
            (Some(Zero), Some(Zero), Some(Unsigned64)),
            (Some(Sign), Some(Sign), Some(Signed64)),
            // A zero-extended next to a sign-extended operand is NOT a
            // widening multiply: the signed side's high half is all ones
            // for a negative value.
            (Some(Zero), Some(Sign), Some(LeftHighZero)),
            (Some(Sign), Some(Zero), Some(RightHighZero)),
            (None, Some(Zero), Some(RightHighZero)),
            (Some(Zero), None, Some(LeftHighZero)),
            (Some(Sign), None, None),
            (None, Some(Sign), None),
            (None, None, None),
        ];
        for (lhs, rhs, plan) in table {
            assert_eq!(I128MulPlan::select(lhs, rhs), plan, "{lhs:?} * {rhs:?}");
        }
    }

    /// Every plan, executed from its staging contract over operands that
    /// satisfy its facts, against `u128::wrapping_mul`.
    #[test]
    fn narrow_multiply_plans_match_wrapping_mul() {
        type Shape = fn(u128) -> u128;
        let full: Shape = |v| v;
        let plans: [(I128MulPlan, Shape, Shape); 4] = [
            (I128MulPlan::Unsigned64, zext, zext),
            (I128MulPlan::Signed64, sext, sext),
            (I128MulPlan::RightHighZero, full, zext),
            (I128MulPlan::LeftHighZero, zext, full),
        ];
        for (plan, shape_lhs, shape_rhs) in plans {
            let asm = plan.body().join("\n");
            for (a, b) in multiply_cases(1000) {
                let (a, b) = (shape_lhs(a), shape_rhs(b));
                let mut machine = match plan {
                    I128MulPlan::Unsigned64 | I128MulPlan::Signed64 => {
                        Machine::with(&[("rax", lo(a)), ("rcx", lo(b))])
                    }
                    I128MulPlan::RightHighZero => {
                        Machine::with(&[("rax", lo(a)), ("rdx", hi(a)), ("rcx", lo(b))])
                    }
                    I128MulPlan::LeftHighZero => {
                        Machine::with(&[("rax", lo(a)), ("rcx", lo(b)), ("rsi", hi(b))])
                    }
                };
                machine.run(&asm);
                assert_eq!(
                    machine.pair(),
                    a.wrapping_mul(b),
                    "{plan:?}: {a:#x} * {b:#x}"
                );
            }
        }
    }

    /// Negative control for the facts: a narrow plan run on operands that do
    /// NOT satisfy its precondition computes a different product, so the
    /// facts are load-bearing and a wrong one would be a miscompile.
    #[test]
    fn narrow_multiply_plans_are_wrong_without_their_facts() {
        let a = (3u128 << 64) | 5;
        let b = (7u128 << 64) | 11;
        for plan in [
            I128MulPlan::Unsigned64,
            I128MulPlan::RightHighZero,
            I128MulPlan::LeftHighZero,
        ] {
            let mut machine = Machine::with(&[
                ("rax", lo(a)),
                ("rdx", hi(a)),
                ("rcx", lo(b)),
                ("rsi", hi(b)),
            ]);
            machine.run(&plan.body().join("\n"));
            assert_ne!(machine.pair(), a.wrapping_mul(b), "{plan:?}");
        }
        // The signed plan against zero-extended operands with the top bit set.
        let (a, b) = (u64::MAX as u128, 2u128);
        let mut machine = Machine::with(&[("rax", lo(a)), ("rcx", lo(b))]);
        machine.run(&I128MulPlan::Signed64.body().join("\n"));
        assert_ne!(machine.pair(), a.wrapping_mul(b));
    }

    fn cast(dest: u32, from_ty: IrType, to_ty: IrType) -> Instruction {
        Instruction::Cast {
            dest: Value(dest),
            src: Operand::Value(Value(900)),
            from_ty,
            to_ty,
        }
    }

    fn wide_binop(dest: u32, op: IrBinOp, rhs: Operand) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op,
            lhs: Operand::Value(Value(901)),
            rhs,
            ty: IrType::U128,
        }
    }

    #[test]
    fn high_half_facts_mirror_the_emitted_definition() {
        use I128High::{Sign, Zero};
        for from in [IrType::U8, IrType::U16, IrType::U32, IrType::U64] {
            for to in [IrType::I128, IrType::U128] {
                assert_eq!(i128_high_of_def(&cast(1, from, to)), Some(Zero), "{from:?}");
            }
        }
        for from in [IrType::I8, IrType::I16, IrType::I32, IrType::I64] {
            for to in [IrType::I128, IrType::U128] {
                assert_eq!(i128_high_of_def(&cast(1, from, to)), Some(Sign), "{from:?}");
            }
        }
        // Not an integer widening: a libcall, a relabel, a pointer, or a
        // cast whose RESULT is not 128 bits wide.
        for (from, to) in [
            (IrType::F64, IrType::I128),
            (IrType::F32, IrType::U128),
            (IrType::F128, IrType::U128),
            (IrType::I128, IrType::U128),
            (IrType::U128, IrType::I128),
            (IrType::Ptr, IrType::U128),
            (IrType::U32, IrType::U64),
            (IrType::U128, IrType::U64),
        ] {
            assert_eq!(
                i128_high_of_def(&cast(1, from, to)),
                None,
                "{from:?} -> {to:?}"
            );
        }

        let amount = |n: i64| Operand::Const(IrConst::I32(n as i32));
        for n in [64, 65, 100, 127] {
            assert_eq!(
                i128_high_of_def(&wide_binop(1, IrBinOp::LShr, amount(n))),
                Some(Zero)
            );
            assert_eq!(
                i128_high_of_def(&wide_binop(1, IrBinOp::AShr, amount(n))),
                Some(Sign)
            );
        }
        // Below 64 the old high half survives in part; at 128 and beyond the
        // emitter takes the variable-shift sequence; a variable amount is
        // unknown; a left shift fills the LOW half.
        for n in [0, 1, 63, 128, 200, -1] {
            assert_eq!(
                i128_high_of_def(&wide_binop(1, IrBinOp::LShr, amount(n))),
                None
            );
            assert_eq!(
                i128_high_of_def(&wide_binop(1, IrBinOp::AShr, amount(n))),
                None
            );
        }
        assert_eq!(
            i128_high_of_def(&wide_binop(1, IrBinOp::LShr, Operand::Value(Value(7)))),
            None
        );
        assert_eq!(
            i128_high_of_def(&wide_binop(1, IrBinOp::Shl, amount(64))),
            None
        );

        let mask = |m: u128| Operand::Const(IrConst::I128(m as i128));
        assert_eq!(
            i128_high_of_def(&wide_binop(1, IrBinOp::And, mask(u64::MAX as u128))),
            Some(Zero)
        );
        assert_eq!(
            i128_high_of_def(&wide_binop(1, IrBinOp::And, mask(0))),
            Some(Zero)
        );
        assert_eq!(
            i128_high_of_def(&wide_binop(1, IrBinOp::And, mask(1 << 64))),
            None
        );
        assert_eq!(
            i128_high_of_def(&wide_binop(1, IrBinOp::And, mask(u128::MAX))),
            None
        );
        // A narrower constant is extended by the pair loader, not by the IR.
        assert_eq!(
            i128_high_of_def(&wide_binop(
                1,
                IrBinOp::And,
                Operand::Const(IrConst::I64(1))
            )),
            None
        );
        // The mask may be the left operand.
        assert_eq!(
            i128_high_of_def(&Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::And,
                lhs: mask(0xffff),
                rhs: Operand::Value(Value(9)),
                ty: IrType::U128,
            }),
            Some(Zero)
        );
        // The same operators at 64 bits say nothing about a 128-bit value.
        assert_eq!(
            i128_high_of_def(&Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::LShr,
                lhs: Operand::Value(Value(9)),
                rhs: amount(64),
                ty: IrType::U64,
            }),
            None
        );
        // Other producers of a 128-bit value.
        assert_eq!(
            i128_high_of_def(&wide_binop(1, IrBinOp::Add, mask(1))),
            None
        );
        assert_eq!(
            i128_high_of_def(&Instruction::Copy {
                dest: Value(1),
                src: Operand::Value(Value(2)),
            }),
            None
        );
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

    /// v1, v2: single-definition zero / sign extensions.  v3: a zero
    /// extension REDEFINED by a Copy in another block (phi-elimination
    /// residue).  v4: a zero extension whose id is also written by a
    /// ParamRef -- a definition kind the emitter's def matcher does not list,
    /// so the matcher alone would call the Cast unique.  v5: `x >> 64`.
    /// v6: two zero extensions under one id.
    fn facts_fixture() -> IrFunction {
        let mut func = IrFunction::new("facts".to_string(), IrType::Void, vec![], false);
        func.blocks.push(block(
            0,
            vec![
                cast(1, IrType::U64, IrType::U128),
                cast(2, IrType::I64, IrType::I128),
                cast(3, IrType::U64, IrType::U128),
                cast(4, IrType::U32, IrType::U128),
                wide_binop(5, IrBinOp::LShr, Operand::Const(IrConst::I32(64))),
                cast(6, IrType::U64, IrType::U128),
            ],
            Terminator::Branch(BlockId(1)),
        ));
        func.blocks.push(block(
            1,
            vec![
                Instruction::Copy {
                    dest: Value(3),
                    src: Operand::Value(Value(5)),
                },
                Instruction::ParamRef {
                    dest: Value(4),
                    param_idx: 0,
                    ty: IrType::U128,
                },
                cast(6, IrType::U8, IrType::U128),
                wide_binop(7, IrBinOp::Add, Operand::Value(Value(1))),
            ],
            Terminator::Return(None),
        ));
        func
    }

    #[test]
    fn high_half_facts_require_exactly_one_definition_of_any_kind() {
        let func = facts_fixture();
        let facts = build_i128_high_facts(&func);
        assert_eq!(facts.get(&1), Some(&I128High::Zero));
        assert_eq!(facts.get(&2), Some(&I128High::Sign));
        assert_eq!(facts.get(&5), Some(&I128High::Zero));
        assert_eq!(facts.get(&3), None, "redefined by a Copy");
        assert_eq!(facts.get(&4), None, "redefined by a ParamRef");
        assert_eq!(facts.get(&6), None, "two widening casts, one id");
        assert_eq!(facts.get(&7), None, "arithmetic proves nothing");
        assert_eq!(facts.len(), 3);
        // The hazard the exhaustive count closes: the def matcher answers
        // "unique Cast" for v4, because ParamRef is not on its list.
        assert!(matches!(
            X86Codegen::find_defining_instruction(&func, 4),
            Some(Instruction::Cast { .. })
        ));
    }

    /// A codegen positioned on `func` with 16-byte slots for the i128 values.
    fn codegen_on(func: &IrFunction, slots: &[(u32, i64)]) -> X86Codegen {
        let mut cg = X86Codegen::new();
        cg.current_func = Some(func as *const IrFunction);
        for &(value, offset) in slots {
            cg.state.value_locations.insert(value, StackSlot(offset));
            cg.state.i128_values.insert(value);
        }
        cg
    }

    #[test]
    fn narrow_multiply_reads_only_the_low_halves_of_two_zero_extensions() {
        let func = facts_fixture();
        let mut cg = codegen_on(&func, &[(1, -32), (5, -64)]);
        assert!(cg.emit_i128_mul_narrow_impl(&Operand::Value(Value(1)), &Operand::Value(Value(5))));
        assert_eq!(
            emitted_lines(&cg),
            ["movq -64(%rbp), %rcx", "movq -32(%rbp), %rax", "mulq %rcx"]
        );
        let asm = cg.state.out.buf.clone();
        for (a, b) in multiply_cases(200) {
            let (a, b) = (zext(a), zext(b));
            let mut machine = Machine::default();
            // Only the low halves are provided: touching a high half is a
            // read of unwritten memory and fails the run.
            machine.slots.insert(-32, lo(a));
            machine.slots.insert(-64, lo(b));
            machine.run(&asm);
            assert_eq!(machine.pair(), a.wrapping_mul(b));
        }
    }

    #[test]
    fn narrow_multiply_drops_one_cross_term_for_a_single_narrow_operand() {
        let func = facts_fixture();
        // v7 (arithmetic, no fact) * v1 (zero-extended): right high zero.
        let mut cg = codegen_on(&func, &[(1, -32), (7, -96)]);
        assert!(cg.emit_i128_mul_narrow_impl(&Operand::Value(Value(7)), &Operand::Value(Value(1))));
        assert_eq!(
            emitted_lines(&cg),
            [
                "movq -32(%rbp), %rcx",
                "movq -96(%rbp), %rax",
                "movq -88(%rbp), %rdx",
                "imulq %rcx, %rdx",
                "movq %rdx, %rsi",
                "mulq %rcx",
                "addq %rsi, %rdx",
            ]
        );
        let right_zero = cg.state.out.buf.clone();

        // v1 * v7: left high zero.
        let mut cg = codegen_on(&func, &[(1, -32), (7, -96)]);
        assert!(cg.emit_i128_mul_narrow_impl(&Operand::Value(Value(1)), &Operand::Value(Value(7))));
        assert_eq!(
            emitted_lines(&cg),
            [
                "movq -96(%rbp), %rcx",
                "movq -88(%rbp), %rsi",
                "movq -32(%rbp), %rax",
                "imulq %rax, %rsi",
                "mulq %rcx",
                "addq %rsi, %rdx",
            ]
        );
        let left_zero = cg.state.out.buf.clone();

        for (narrow, full) in multiply_cases(200) {
            let narrow = zext(narrow);
            for asm in [&right_zero, &left_zero] {
                let mut machine = Machine::default();
                machine.slots.insert(-32, lo(narrow));
                machine.slots.insert(-96, lo(full));
                machine.slots.insert(-88, hi(full));
                machine.run(asm);
                assert_eq!(machine.pair(), narrow.wrapping_mul(full));
            }
        }
    }

    /// No fact, no narrow form -- and NOTHING emitted, which is the trait
    /// contract the generic fallback relies on.
    #[test]
    fn narrow_multiply_declines_without_emitting() {
        let func = facts_fixture();
        let slots = [(1, -32), (2, -48), (3, -64), (4, -80), (6, -112), (7, -96)];
        for (lhs, rhs, why) in [
            (7u32, 7u32, "no facts at all"),
            (3, 3, "a Copy redefines the value"),
            (4, 4, "a ParamRef redefines the value"),
            (6, 6, "two definitions"),
            (2, 7, "a lone sign extension keeps its cross term"),
            (7, 2, "a lone sign extension keeps its cross term"),
        ] {
            let mut cg = codegen_on(&func, &slots);
            assert!(
                !cg.emit_i128_mul_narrow_impl(
                    &Operand::Value(Value(lhs)),
                    &Operand::Value(Value(rhs))
                ),
                "{why}"
            );
            assert!(cg.state.out.buf.is_empty(), "{why}: {}", cg.state.out.buf);
        }

        // A fact about a value that is not a slot-homed 128-bit value here
        // (no slot; not in the i128 registry; an alloca) is not usable.
        let mut cg = codegen_on(&func, &[(1, -32)]);
        assert!(
            !cg.emit_i128_mul_narrow_impl(&Operand::Value(Value(5)), &Operand::Value(Value(5)))
        );
        let mut cg = codegen_on(&func, &[]);
        cg.state.value_locations.insert(1, StackSlot(-32));
        assert!(
            !cg.emit_i128_mul_narrow_impl(&Operand::Value(Value(1)), &Operand::Value(Value(1)))
        );
        // Constants never carry a slot.
        let mut cg = codegen_on(&func, &[(7, -96)]);
        assert!(!cg.emit_i128_mul_narrow_impl(
            &Operand::Value(Value(7)),
            &Operand::Const(IrConst::I128(3))
        ));
        assert!(cg.state.out.buf.is_empty());

        // The facts answer for the function they were built from only.
        let other = IrFunction::new("other".to_string(), IrType::Void, vec![], false);
        let mut cg = codegen_on(&func, &[(1, -32), (5, -64)]);
        assert_eq!(cg.i128_high_fact(1), Some(I128High::Zero));
        cg.current_func = Some(&other as *const IrFunction);
        assert_eq!(cg.i128_high_fact(1), None);
    }

    #[test]
    fn signed_narrow_multiply_is_the_one_operand_form() {
        let func = facts_fixture();
        let mut cg = codegen_on(&func, &[(2, -48)]);
        assert!(cg.emit_i128_mul_narrow_impl(&Operand::Value(Value(2)), &Operand::Value(Value(2))));
        assert_eq!(
            emitted_lines(&cg),
            ["movq -48(%rbp), %rcx", "movq -48(%rbp), %rax", "imulq %rcx"]
        );
        let asm = cg.state.out.buf.clone();
        for (a, _) in multiply_cases(200) {
            let a = sext(a);
            let mut machine = Machine::default();
            machine.slots.insert(-48, lo(a));
            machine.run(&asm);
            assert_eq!(machine.pair(), a.wrapping_mul(a));
        }
    }

    /// The constant multiplier, in both forms, for every combination of a
    /// small / wide low half and an absent / present high half.
    #[test]
    fn constant_i128_multiply_matches_wrapping_mul() {
        let func = facts_fixture();
        let multipliers: [u128; 8] = [
            3,
            0x7fff_ffff,
            0x8000_0000,
            0x9E37_79B9_7F4A_7C15,
            u64::MAX as u128,
            (5u128 << 64) | 7,
            (0xDEAD_BEEF_0000_0001u128 << 64) | 0xFFFF_FFFF_0000_0003,
            1u128 << 64,
        ];
        for multiplier in multipliers {
            let (mlo, mhi) = (lo(multiplier), hi(multiplier));
            let rhs = Operand::Const(IrConst::I128(multiplier as i128));

            // General left operand (v7): both halves are read.
            let mut cg = codegen_on(&func, &[(7, -96)]);
            cg.emit_i128_mul_const_impl(&Operand::Value(Value(7)), &rhs, mlo, mhi);
            let general = cg.state.out.buf.clone();
            assert!(general.contains("movq -88(%rbp), %rdx"), "{general}");
            assert_eq!(general.contains("imulq %r8, %rsi"), mhi != 0, "{general}");

            // Zero-extended left operand (v1): the lhs.hi term is gone, and
            // so is the read of the high half.
            let mut cg = codegen_on(&func, &[(1, -32)]);
            cg.emit_i128_mul_const_impl(&Operand::Value(Value(1)), &rhs, mlo, mhi);
            let narrow = cg.state.out.buf.clone();
            assert!(!narrow.contains("-24(%rbp)"), "{narrow}");
            assert!(!narrow.contains("%rcx"), "{narrow}");
            if mhi == 0 {
                assert_eq!(emitted_lines(&cg).len(), 3, "{narrow}");
            }

            for (a, _) in multiply_cases(200) {
                let mut machine = Machine::default();
                machine.slots.insert(-96, lo(a));
                machine.slots.insert(-88, hi(a));
                machine.run(&general);
                assert_eq!(machine.pair(), a.wrapping_mul(multiplier), "{a:#x}");

                let a = zext(a);
                let mut machine = Machine::default();
                machine.slots.insert(-32, lo(a));
                machine.run(&narrow);
                assert_eq!(machine.pair(), a.wrapping_mul(multiplier), "{a:#x}");
            }
        }
    }
}
