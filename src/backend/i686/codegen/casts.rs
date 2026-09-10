//! i686 numeric cast emission.
//!
//! # Representation conventions (this backend)
//!
//! - Scalar integers and F32 bit patterns use `%eax`; signed subword values
//!   are sign-extended in the register, unsigned subword values are
//!   zero-extended (`movsbl`/`movswl`/`movzbl`/`movzwl`).
//! - I64/U64 and raw F64 copies use the `%edx:%eax` pair.
//! - F128 denotes native x87 extended precision (80-bit) inside a 12-byte
//!   ABI object; `fstpt` writes its 10 significant bytes. Values marked in
//!   `f128_direct_slots` hold that native representation in a 16-byte
//!   stack slot.
//!
//! # ISA floor
//!
//! The sequences below use the scalar SSE conversions (`cvtsi2ssl`,
//! `cvttss2si`, `movd`) and SSE3's `fisttp`. That is not a new
//! requirement: the i686 backend already emits SSE/SSE2/SSE3
//! unconditionally across `intrinsics.rs`, `alu.rs`, `comparison.rs` and
//! the original cast paths, and no target-feature gating interface exists
//! (recorded as a follow-up; the Linux-kernel `no_sse` flag only governs
//! variadic prologues). `fisttp` truncates toward zero by definition, so
//! float-to-integer conversion is independent of the ambient x87
//! rounding control without any control-word save/restore.
//!
//! # Unsigned conversions (no ambient-precision arithmetic)
//!
//! - U64 → x87: values with bit 63 set are loaded with `fldt` from their
//!   exact native extended encoding (significand = the 64 integer bits,
//!   exponent word `0x403e` = bias + 63), not via `fildq` + `fadds 2^64`.
//!   The correction addition is exact only while the x87 precision
//!   control is 64-bit; the encoding is exact at every PC setting.
//! - x87 → U64: compare against `2^63` (`fucomip`), convert the low half
//!   with `fisttpq` directly, and for `x >= 2^63` convert the exact
//!   difference `x - 2^63` (Sterbenz-exact in extended precision, result
//!   in `[0, 2^63)`) and OR bit 63 back. No signed-overflow indefinite,
//!   no FE_INVALID on valid inputs, no control-word traffic.
//! - F32 → U32: `flds` + `fisttpq` (64-bit form), never the signed
//!   `cvttss2si`, which returns the indefinite `0x80000000` and raises
//!   FE_INVALID for `[2^31, 2^32)`.
//!
//! # x87 stack discipline
//!
//! Every x87-backed conversion below pushes exactly the entries it pops.
//! `fstpt` is store-and-pop, so an F128 destination without a stack slot
//! (a dead result under this backend's convention; see `float_ops.rs`)
//! must skip the whole conversion before any load — otherwise the load
//! leaks one stack entry per dead cast. F32/F64 destinations cannot
//! leak: their store paths always pop (`fstps`, or `emit_f64_store_from_x87`,
//! which pops even without a slot).
//!
//! # Cache discipline
//!
//! `operand_to_eax` records the loaded value in the accumulator cache, so
//! every instruction that writes `%eax` (or `%edx`) to something other
//! than the cached value must invalidate the corresponding entry.
//! Pure-x87 and push/pop sequences leave `%eax` untouched and therefore
//! keep the cache entry alive.

use super::emit::I686Codegen;
use crate::backend::traits::ArchCodegen;
use crate::common::types::IrType;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

/// Positive native extended-precision numbers in `[2^63, 2^64)` have this
/// complete sign/exponent word (bias 16383 + exponent 63).
const U64_HIGH_X87_EXPONENT: u16 = 0x403e;

fn cast_is_float(ty: IrType) -> bool {
    matches!(ty, IrType::F32 | IrType::F64 | IrType::F128)
}

fn cast_is_integer(ty: IrType) -> bool {
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
    )
}

fn cast_is_integer_pair(ty: IrType) -> bool {
    matches!(ty, IrType::I64 | IrType::U64)
}

/// Establish the canonical 32-bit register form of a subword integer
/// already in eax.
fn cast_normalization_instruction(ty: IrType) -> Option<&'static str> {
    match ty {
        IrType::I8 => Some("    movsbl %al, %eax"),
        IrType::U8 => Some("    movzbl %al, %eax"),
        IrType::I16 => Some("    movswl %ax, %eax"),
        IrType::U16 => Some("    movzwl %ax, %eax"),
        _ => None,
    }
}

impl I686Codegen {
    pub(super) fn emit_cast_impl(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) {
        use crate::backend::cast::{CastKind, classify_cast_with_f128};

        // Preserve the existing i128 delegation: its implementation and
        // calling convention live outside this file.
        if crate::backend::generation::is_i128_type(from_ty)
            || crate::backend::generation::is_i128_type(to_ty)
        {
            crate::backend::traits::emit_cast_default(self, dest, src, from_ty, to_ty);
            return;
        }

        // A genuine identity operation on the same SSA value needs no
        // code. Same-register coalescing is restricted to 32-bit
        // same-width pairs: it requires no assumptions about canonical
        // subword register contents (every subword producer normalizes,
        // but the invariant is not audited across all consumers).
        if let Operand::Value(source) = src {
            if source.0 == dest.0 && from_ty == to_ty {
                return;
            }
            if matches!(from_ty, IrType::I32 | IrType::U32)
                && matches!(to_ty, IrType::I32 | IrType::U32)
            {
                if let (Some(&dp), Some(&sp)) = (
                    self.reg_assignments.get(&dest.0),
                    self.reg_assignments.get(&source.0),
                ) {
                    if dp == sp {
                        return;
                    }
                }
            }
        }

        // Identity casts must preserve the complete representation.
        // In particular F64 and F128 must never reach the scalar eax
        // fallback, which would truncate them to four bytes.
        if from_ty == to_ty {
            match to_ty {
                IrType::F32 => {
                    self.operand_to_eax(src);
                    self.store_eax_to(dest);
                    return;
                }
                IrType::F64 => {
                    self.emit_load_acc_pair(src);
                    self.state.reg_cache.invalidate_all();
                    self.emit_store_acc_pair(dest);
                    self.state.reg_cache.invalidate_all();
                    return;
                }
                IrType::F128 => {
                    // Dead-destination contract: skip before any load.
                    if self.state.get_slot(dest.0).is_none() {
                        return;
                    }
                    // A direct-slot source holds the native encoding:
                    // copy the 12-byte ABI object without touching the
                    // FPU (NaN payloads and padding survive verbatim).
                    if self.cast_copy_direct_f128(dest, src) {
                        return;
                    }
                    // Non-direct operands go through the existing
                    // representation-aware materialization helper.
                    self.emit_f128_load_to_x87(src);
                    self.cast_store_x87_float(dest, to_ty);
                    return;
                }
                _ => {}
            }
        }

        let kind = classify_cast_with_f128(from_ty, to_ty, true);

        // --- Scalar float<->integer casts that fit the single-register
        // interface (the SSE fast paths for the common sizes) ---
        //
        // The width restriction is stated directly in the types: only
        // F32<->subword/integer32 conversions use the eax interface. F64
        // and F128 (and every wide pair) take the x87 path below; the
        // previous classifier-boolean form derived the same set through
        // `to_f64`/`from_f64` encodings, which did not locally prove the
        // width.
        let integer_to_f32 =
            to_ty == IrType::F32 && cast_is_integer(from_ty) && !cast_is_integer_pair(from_ty);

        let f32_to_integer =
            from_ty == IrType::F32 && cast_is_integer(to_ty) && !cast_is_integer_pair(to_ty);

        if integer_to_f32 || f32_to_integer {
            self.operand_to_eax(src);
            self.emit_cast_instrs_impl(from_ty, to_ty);
            self.store_eax_to(dest);
            return;
        }

        // --- Everything involving floating point and a wide operand,
        // F64, or F128 goes through x87 ---
        if cast_is_float(from_ty) || cast_is_float(to_ty) {
            assert!(
                cast_is_float(from_ty) || cast_is_integer(from_ty),
                "unsupported i686 cast source type"
            );
            assert!(
                cast_is_float(to_ty) || cast_is_integer(to_ty),
                "unsupported i686 cast destination type"
            );

            // Dead-destination contract: diagnose before emitting a load
            // that pushes st(0).
            if to_ty == IrType::F128 && self.state.get_slot(dest.0).is_none() {
                return;
            }

            self.cast_load_x87(src, from_ty);

            if cast_is_float(to_ty) {
                self.cast_store_x87_float(dest, to_ty);
            } else {
                self.cast_x87_to_integer(to_ty);

                if cast_is_integer_pair(to_ty) {
                    self.emit_store_acc_pair(dest);
                    self.state.reg_cache.invalidate_all();
                } else {
                    self.store_eax_to(dest);
                }
            }

            return;
        }

        // --- Integer <-> integer ---

        // Same-width 64-bit signedness changes copy the whole payload.
        if cast_is_integer_pair(from_ty) && cast_is_integer_pair(to_ty) {
            // Planner contract inherited from the original implementation:
            // a virtual mul-acc result has no independently materialized
            // consumer, so its no-op must emit nothing.
            if self.mulacc_virtual_casts.contains(&dest.0) {
                return;
            }

            self.emit_load_acc_pair(src);
            self.state.reg_cache.invalidate_all();
            self.emit_store_acc_pair(dest);
            self.state.reg_cache.invalidate_all();
            return;
        }

        match kind {
            CastKind::IntWiden { .. } if cast_is_integer_pair(to_ty) => {
                if self.mulacc_virtual_casts.contains(&dest.0) {
                    return;
                }

                self.operand_to_eax(src);
                self.cast_normalize_eax(from_ty);
                self.state.reg_cache.invalidate_all();

                if from_ty.is_signed() {
                    self.state.emit("    cltd");
                } else {
                    self.state.emit("    xorl %edx, %edx");
                }

                self.emit_store_acc_pair(dest);
                self.state.reg_cache.invalidate_all();
            }

            CastKind::IntNarrow { .. } if cast_is_integer_pair(from_ty) => {
                // Only the low word survives truncation, but the pair
                // loader is the documented interface for wide operands
                // (it knows how to reach every wide representation).
                self.emit_load_acc_pair(src);
                self.state.reg_cache.invalidate_all();
                self.cast_normalize_eax(to_ty);
                self.store_eax_to(dest);
            }

            _ => {
                self.operand_to_eax(src);
                self.emit_cast_instrs_impl(from_ty, to_ty);
                self.store_eax_to(dest);
            }
        }
    }

    /// Normalize a subword integer in EAX.
    ///
    /// EAX may no longer contain the previously cached representation.
    fn cast_normalize_eax(&mut self, ty: IrType) {
        if let Some(instruction) = cast_normalization_instruction(ty) {
            self.state.reg_cache.invalidate_acc();
            self.state.emit(instruction);
        }
    }

    /// Copy a known native F128 slot without floating-point arithmetic.
    ///
    /// The full 12-byte ABI object is copied (3 dwords), so NaN payloads
    /// and padding survive verbatim. All source words are read before any
    /// destination word is written, so overlapping source and destination
    /// slots cannot corrupt the copy. No x87 round trip, no stack-engine
    /// traffic, no precision-control interaction.
    ///
    /// %eax/%ecx/%edx scratch contract: the i686 register allocator's
    /// scratch-hazard model (regalloc.rs collect_i686_scratch_hazard_points)
    /// marks every Cast instruction as an ECX/EDX hazard point, so no value
    /// homed in a caller-saved register can be live across this sequence —
    /// the scratch use here is allocator-modelled, not cache-hidden.
    fn cast_copy_direct_f128(&mut self, dest: &Value, src: &Operand) -> bool {
        let source = match src {
            Operand::Value(source) => source,
            _ => return false,
        };

        if !self.state.f128_direct_slots.contains(&source.0) {
            return false;
        }

        let source_ref = match self.state.get_slot(source.0) {
            Some(slot) => self.slot_ref(slot),
            None => return false,
        };
        let destination_ref = match self.state.get_slot(dest.0) {
            Some(slot) => self.slot_ref(slot),
            None => return false,
        };

        let source_slot = self.state.get_slot(source.0).expect("checked above");
        let destination_slot = self.state.get_slot(dest.0).expect("checked above");
        let ssr4 = self.slot_ref_offset(source_slot, 4);
        let ssr8 = self.slot_ref_offset(source_slot, 8);
        let dsr4 = self.slot_ref_offset(destination_slot, 4);
        let dsr8 = self.slot_ref_offset(destination_slot, 8);

        self.state.reg_cache.invalidate_all();

        emit!(self.state, "    movl {}, %eax", source_ref);
        emit!(self.state, "    movl {}, %ecx", ssr4);
        emit!(self.state, "    movl {}, %edx", ssr8);
        emit!(self.state, "    movl %eax, {}", destination_ref);
        emit!(self.state, "    movl %ecx, {}", dsr4);
        emit!(self.state, "    movl %edx, {}", dsr8);

        self.state.f128_direct_slots.insert(dest.0);
        true
    }

    /// Load one supported numeric operand into st(0).
    ///
    /// Operand materialization may modify EAX/EDX. Integer-pair loads
    /// invalidate cached register contents here; subword normalization
    /// invalidates the accumulator cache when it emits an instruction.
    fn cast_load_x87(&mut self, src: &Operand, from_ty: IrType) {
        match from_ty {
            IrType::F32 => {
                self.operand_to_eax(src);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }

            IrType::F64 => {
                self.emit_f64_load_to_x87(src);
            }

            IrType::F128 => {
                self.emit_f128_load_to_x87(src);
            }

            IrType::I64 => {
                self.emit_load_acc_pair(src);
                self.state.reg_cache.invalidate_all();
                self.state.emit("    pushl %edx");
                self.state.emit("    pushl %eax");
                self.state.emit("    fildq (%esp)");
                self.state.emit("    addl $8, %esp");
            }

            IrType::U64 => {
                self.emit_load_acc_pair(src);
                self.state.reg_cache.invalidate_all();
                self.cast_u64_to_x87();
            }

            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 => {
                self.operand_to_eax(src);
                self.cast_normalize_eax(from_ty);

                if from_ty == IrType::U32 {
                    // Branch-free zero extension into a signed i64.
                    self.state.emit("    pushl $0");
                    self.state.emit("    pushl %eax");
                    self.state.emit("    fildq (%esp)");
                    self.state.emit("    addl $8, %esp");
                } else {
                    // Canonical U8/U16 values also fit signed i32.
                    self.state.emit("    pushl %eax");
                    self.state.emit("    fildl (%esp)");
                    self.state.emit("    addl $4, %esp");
                }
            }

            _ => panic!("unsupported i686 numeric cast source"),
        }
    }

    /// Load a U64 exactly, independently of the x87 precision control.
    ///
    /// Called with the value in `%edx:%eax`. For a high-bit-set value the
    /// native extended encoding is exact:
    ///
    /// ```text
    /// sign        = 0
    /// exponent    = bias + 63 = 0x403e
    /// significand = the unsigned integer bits
    /// ```
    ///
    /// No floating-point correction addition is involved, so no PC
    /// setting can round it.
    fn cast_u64_to_x87(&mut self) {
        let high = self.state.fresh_label("cast_u64_high");
        let done = self.state.fresh_label("cast_u64_loaded");

        self.state.emit("    subl $12, %esp");
        self.state.emit("    movl %eax, (%esp)");
        self.state.emit("    movl %edx, 4(%esp)");

        self.state.emit("    testl %edx, %edx");
        self.state.out.emit_jcc_label("    js", &high);

        self.state.emit("    fildq (%esp)");
        self.state.out.emit_jmp_label(&done);

        self.state.out.emit_named_label(&high);
        // The exponent word is needed only on this path: the common
        // low-range conversion executes one fewer store.
        emit!(self.state, "    movw ${}, 8(%esp)", U64_HIGH_X87_EXPONENT);
        self.state.emit("    fldt (%esp)");

        self.state.out.emit_named_label(&done);
        self.state.emit("    addl $12, %esp");
    }

    /// Pop st(0), storing the requested floating-point destination.
    ///
    /// The F128 destination slot was checked by the caller (dead-dest
    /// contract); F32/F64 paths always pop.
    fn cast_store_x87_float(&mut self, dest: &Value, to_ty: IrType) {
        match to_ty {
            IrType::F32 => {
                self.state.emit("    subl $4, %esp");
                self.state.emit("    fstps (%esp)");
                self.state.emit("    movl (%esp), %eax");
                self.state.emit("    addl $4, %esp");
                self.state.reg_cache.invalidate_acc();
                self.store_eax_to(dest);
            }

            IrType::F64 => {
                self.emit_f64_store_from_x87(dest);
            }

            IrType::F128 => {
                let destination_ref = match self.state.get_slot(dest.0) {
                    Some(slot) => self.slot_ref(slot),
                    None => panic!("i686 F128 cast requires a destination stack slot"),
                };

                // FSTPT writes the 10 significant bytes; the ABI's
                // remaining padding bytes are unspecified.
                emit!(self.state, "    fstpt {}", destination_ref);
                self.state.f128_direct_slots.insert(dest.0);
            }

            _ => panic!("x87 floating store requires a floating-point type"),
        }
    }

    /// Pop st(0), returning an integer in eax or edx:eax.
    ///
    /// FISTTP truncates toward zero without touching the caller's x87
    /// control word, so the result does not depend on the ambient
    /// rounding direction.
    ///
    /// This implements ordinary C conversions on their defined domain. It
    /// does not implement saturating conversions or assign semantics to
    /// NaNs, infinities, or out-of-range truncated results.
    fn cast_x87_to_integer(&mut self, to_ty: IrType) {
        assert!(
            cast_is_integer(to_ty),
            "x87 integer conversion requires an integer destination"
        );

        if to_ty == IrType::U64 {
            self.cast_x87_to_u64();
            return;
        }

        let pair_result = to_ty == IrType::I64;
        let wide_conversion = pair_result || to_ty == IrType::U32;

        if wide_conversion {
            self.state.emit("    subl $8, %esp");
            self.state.emit("    fisttpq (%esp)");
            self.state.emit("    movl (%esp), %eax");
            if pair_result {
                self.state.emit("    movl 4(%esp), %edx");
            }
            self.state.emit("    addl $8, %esp");
            self.state.reg_cache.invalidate_acc();
            if pair_result {
                self.state.reg_cache.invalidate_sec();
            }
        } else {
            // Every defined I8/U8/I16/U16/I32 result fits signed i32.
            self.state.emit("    subl $4, %esp");
            self.state.emit("    fisttpl (%esp)");
            self.state.emit("    movl (%esp), %eax");
            self.state.emit("    addl $4, %esp");
            self.state.reg_cache.invalidate_acc();
        }

        self.cast_normalize_eax(to_ty);
    }

    /// Pop st(0), returning U64 in edx:eax.
    ///
    /// The threshold compare selects between two EXACT conversions, and
    /// neither path performs any x87 arithmetic, so the result does not
    /// depend on the ambient precision-control setting:
    ///
    /// ```text
    /// x <  2^63: fisttpq(x) directly — truncation, in signed range.
    /// x >= 2^63: the native extended encoding is stored with fstpt.
    ///            Every finite extended value in [2^63, 2^64) is an
    ///            integer whose sign/exponent word is 0x403e and whose
    ///            64-bit significand IS the unsigned result — read the
    ///            payload directly, no subtraction.
    /// ```
    ///
    /// The previous high path computed `fisttpq(x - 2^63)` and XORed the
    /// high bit back in. That subtraction rounds at the ambient x87
    /// precision: under a 24- or 53-bit precision control, 2^64 - 1
    /// (exactly representable in extended precision, e.g. loaded with
    /// fldt from memory or produced by fild-based integer arithmetic)
    /// rounds to 2^63, which lies outside the signed 64-bit range and
    /// converts to the integer-indefinite value — 0 after the XOR
    /// instead of UINT64_MAX (verified against GCC 14.2 -m32, which
    /// miscompiles identically; we exceed it).
    ///
    /// NaNs, infinities and values >= 2^64 fall through to fisttpq,
    /// yielding the same integer-indefinite value GCC produces.
    /// The threshold constant is 2^63 exactly as an F32 (`0x5f000000`),
    /// so the compare needs no double-width load; FUCOMIP pops the
    /// threshold and keeps x, and the unordered (NaN) case sets CF=ZF=1
    /// so `jbe` routes it to the high-path fallback.
    fn cast_x87_to_u64(&mut self) {
        let high = self.state.fresh_label("cast_fp_u64_high");
        let not_range = self.state.fresh_label("cast_fp_u64_notrange");
        let done = self.state.fresh_label("cast_fp_u64_done");

        self.state.emit("    subl $12, %esp");
        self.state.emit("    movl $0x5f000000, (%esp)");
        self.state.emit("    flds (%esp)");
        self.state.emit("    fucomip %st(1), %st");
        self.state.out.emit_jcc_label("    jbe", &high);

        // Low path: defined inputs truncate into signed i64. FISTTP
        // truncates toward zero independently of the ambient rounding
        // direction and performs no arithmetic subject to precision
        // control. Fractional inputs may still raise FE_INEXACT.
        self.state.emit("    fisttpq (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    movl 4(%esp), %edx");
        self.state.out.emit_jmp_label(&done);

        self.state.out.emit_named_label(&high);
        // High path: store the native extended representation (stores are
        // exact) and inspect the sign/exponent word without reloading.
        self.state.emit("    fstpt (%esp)");
        emit!(self.state, "    cmpw ${}, 8(%esp)", U64_HIGH_X87_EXPONENT);
        self.state.out.emit_jcc_label("    jne", &not_range);

        // Finite [2^63, 2^64): the significand is the result.
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    movl 4(%esp), %edx");
        self.state.out.emit_jmp_label(&done);

        // NaN / infinity / >= 2^64 (or negative): integer-indefinite.
        self.state.out.emit_named_label(&not_range);
        self.state.emit("    fldt (%esp)");
        self.state.emit("    fisttpq (%esp)");
        self.state.emit("    movl (%esp), %eax");
        self.state.emit("    movl 4(%esp), %edx");

        self.state.out.emit_named_label(&done);
        self.state.emit("    addl $12, %esp");
        self.state.reg_cache.invalidate_acc();
        self.state.reg_cache.invalidate_sec();
    }

    /// Emit a scalar cast with the operand already in eax, result in eax.
    ///
    /// Wide integer and F64/F128 conversions are handled by
    /// `emit_cast_impl`, not by this single-register interface.
    /// Unsupported requests must not silently become no-ops.
    pub(super) fn emit_cast_instrs_impl(&mut self, from_ty: IrType, to_ty: IrType) {
        use crate::backend::cast::{CastKind, classify_cast};

        assert!(
            !cast_is_integer_pair(from_ty)
                && !cast_is_integer_pair(to_ty)
                && !matches!(from_ty, IrType::F64 | IrType::F128)
                && !matches!(to_ty, IrType::F64 | IrType::F128)
                && !crate::backend::generation::is_i128_type(from_ty)
                && !crate::backend::generation::is_i128_type(to_ty),
            "wide i686 cast reached the scalar cast emitter"
        );

        match classify_cast(from_ty, to_ty) {
            CastKind::Noop => {}

            // Same-width signedness changes must re-canonicalize the
            // register: `(I8)(U8)255` is -1, i.e. 0xffffffff, not the
            // zero-extended 0x000000ff the unsigned producer left behind.
            CastKind::IntNarrow { .. }
            | CastKind::SignedToUnsignedSameSize { .. }
            | CastKind::UnsignedToSignedSameSize { .. } => {
                self.state.reg_cache.invalidate_acc();
                self.cast_normalize_eax(to_ty);
            }

            CastKind::IntWiden { .. } => {
                self.state.reg_cache.invalidate_acc();
                self.cast_normalize_eax(from_ty);

                // Widening a negative signed source to U16 (or U8)
                // requires the modulo-2^width value: I8(-1) -> U16 is
                // 0x0000ffff, but sign extension alone leaves
                // 0xffffffff. Re-canonicalize for the destination width.
                if from_ty.is_signed() && matches!(to_ty, IrType::U8 | IrType::U16) {
                    self.cast_normalize_eax(to_ty);
                }
            }

            CastKind::SignedToFloat { to_f64: false, .. } => {
                self.state.reg_cache.invalidate_acc();
                self.cast_normalize_eax(from_ty);
                self.state.emit("    cvtsi2ssl %eax, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
            }

            CastKind::UnsignedToFloat { to_f64: false, .. } => {
                self.state.reg_cache.invalidate_acc();
                self.cast_normalize_eax(from_ty);

                if matches!(from_ty, IrType::U8 | IrType::U16) {
                    // Canonical values always fit signed i32.
                    self.state.emit("    cvtsi2ssl %eax, %xmm0");
                    self.state.emit("    movd %xmm0, %eax");
                    return;
                }

                let high = self.state.fresh_label("cast_u32_f32_high");
                let done = self.state.fresh_label("cast_u32_f32_done");

                self.state.emit("    testl %eax, %eax");
                self.state.out.emit_jcc_label("    js", &high);

                self.state.emit("    cvtsi2ssl %eax, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.out.emit_jmp_label(&done);

                self.state.out.emit_named_label(&high);
                // Exact 64-bit integer load, one F32 rounding at store.
                // No shift/round/add reconstruction, whose directed
                // rounding can disagree with a single correctly rounded
                // conversion.
                self.state.emit("    pushl $0");
                self.state.emit("    pushl %eax");
                self.state.emit("    fildq (%esp)");
                self.state.emit("    fstps (%esp)");
                self.state.emit("    popl %eax");
                self.state.emit("    addl $4, %esp");

                self.state.out.emit_named_label(&done);
            }

            CastKind::FloatToSigned { from_f64: false } => {
                self.state.reg_cache.invalidate_acc();
                self.state.emit("    movd %eax, %xmm0");
                self.state.emit("    cvttss2si %xmm0, %eax");
                self.cast_normalize_eax(to_ty);
            }

            CastKind::FloatToUnsigned {
                from_f64: false,
                to_u64: false,
            } => {
                self.state.reg_cache.invalidate_acc();

                if matches!(to_ty, IrType::U8 | IrType::U16) {
                    // All defined results fit the signed SSE conversion.
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    cvttss2si %xmm0, %eax");
                    self.cast_normalize_eax(to_ty);
                } else {
                    assert!(
                        to_ty == IrType::U32,
                        "unsupported scalar unsigned floating cast"
                    );

                    // Never the signed 32-bit cvttss2si: for [2^31, 2^32)
                    // it returns the indefinite 0x80000000 and raises
                    // FE_INVALID. The signed 64-bit conversion accommodates
                    // every defined U32 result, including [2^31, 2^32).
                    // Fractional inputs may still raise FE_INEXACT. The F32
                    // bits are consumed by flds before the conversion
                    // overwrites the scratch.
                    self.state.emit("    subl $8, %esp");
                    self.state.emit("    movl %eax, (%esp)");
                    self.state.emit("    flds (%esp)");
                    self.state.emit("    fisttpq (%esp)");
                    self.state.emit("    movl (%esp), %eax");
                    self.state.emit("    addl $8, %esp");
                }
            }

            // A missing lowering must not silently become a no-op.
            _ => panic!("unsupported i686 scalar cast classification"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        U64_HIGH_X87_EXPONENT, cast_is_float, cast_is_integer, cast_is_integer_pair,
        cast_normalization_instruction,
    };
    use crate::common::types::IrType;

    const X87_EXPONENT_BIAS: i32 = 16383;
    const TWO63: f64 = 9_223_372_036_854_775_808.0;
    const TWO64: f64 = 18_446_744_073_709_551_616.0;

    /// Construct the representation the high-U64 load path emits.
    fn encode_high_u64(value: u64) -> [u8; 10] {
        assert!(value >= (1u64 << 63));

        let mut bytes = [0u8; 10];
        bytes[..8].copy_from_slice(&value.to_le_bytes());
        bytes[8..].copy_from_slice(&U64_HIGH_X87_EXPONENT.to_le_bytes());
        bytes
    }

    /// Decode a positive normal extended value known to represent an
    /// integer, using integer arithmetic rather than host floating point.
    fn decode_positive_integer(bytes: [u8; 10]) -> u128 {
        let mut significand_bytes = [0u8; 8];
        significand_bytes.copy_from_slice(&bytes[..8]);
        let significand = u64::from_le_bytes(significand_bytes);

        let sign_exponent = u16::from_le_bytes([bytes[8], bytes[9]]);
        assert_eq!(sign_exponent & 0x8000, 0);
        assert_ne!(significand & (1u64 << 63), 0);

        let exponent = i32::from(sign_exponent) - X87_EXPONENT_BIAS;
        let shift = exponent - 63;

        if shift >= 0 {
            u128::from(significand) << (shift as u32)
        } else {
            let right_shift = (-shift) as u32;
            assert!(right_shift < 64);
            assert_eq!(significand & ((1u64 << right_shift) - 1), 0);
            u128::from(significand >> right_shift)
        }
    }

    /// Arithmetic model of the finite-input U64 conversion split.
    fn split_f64_to_u64(value: f64) -> u64 {
        assert!(value.is_finite());
        assert!(value > -1.0 && value < TWO64);

        if value >= TWO63 {
            ((value - TWO63) as i64 as u64) ^ (1u64 << 63)
        } else {
            value as i64 as u64
        }
    }

    fn next_u64(state: &mut u64) -> u64 {
        let mut value = *state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        *state = value;
        value
    }

    #[test]
    fn canonical_register_instruction_selection() {
        for (ty, expected) in [
            (IrType::I8, "    movsbl %al, %eax"),
            (IrType::U8, "    movzbl %al, %eax"),
            (IrType::I16, "    movswl %ax, %eax"),
            (IrType::U16, "    movzwl %ax, %eax"),
        ] {
            assert_eq!(cast_normalization_instruction(ty), Some(expected));
        }

        for ty in [
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
            IrType::F32,
            IrType::F64,
            IrType::F128,
        ] {
            assert_eq!(cast_normalization_instruction(ty), None);
        }
    }

    #[test]
    fn numeric_type_classification() {
        for ty in [
            IrType::I8,
            IrType::U8,
            IrType::I16,
            IrType::U16,
            IrType::I32,
            IrType::U32,
            IrType::I64,
            IrType::U64,
        ] {
            assert!(cast_is_integer(ty));
            assert!(!cast_is_float(ty));
        }

        for ty in [IrType::F32, IrType::F64, IrType::F128] {
            assert!(cast_is_float(ty));
            assert!(!cast_is_integer(ty));
            assert!(!cast_is_integer_pair(ty));
        }

        assert!(cast_is_integer_pair(IrType::I64));
        assert!(cast_is_integer_pair(IrType::U64));
        assert!(!cast_is_integer_pair(IrType::I32));
        assert!(!cast_is_integer_pair(IrType::U32));
    }

    #[test]
    fn high_unsigned_exponent_is_bias_plus_63() {
        assert_eq!(i32::from(U64_HIGH_X87_EXPONENT), X87_EXPONENT_BIAS + 63);
        assert_eq!(U64_HIGH_X87_EXPONENT & 0x8000, 0);
    }

    #[test]
    fn high_unsigned_representation_boundaries() {
        for value in [
            1u64 << 63,
            (1u64 << 63) + 1,
            (1u64 << 63) + 1023,
            (1u64 << 63) + 1024,
            (1u64 << 63) + 2047,
            3u64 << 62,
            u64::MAX - 1,
            u64::MAX,
        ] {
            let bytes = encode_high_u64(value);
            assert_eq!(decode_positive_integer(bytes), u128::from(value));

            let mut payload = [0u8; 8];
            payload.copy_from_slice(&bytes[..8]);
            assert_eq!(u64::from_le_bytes(payload), value);
        }
    }

    /// Integer-arithmetic model of rounding a positive integer to a
    /// binary precision using round-to-nearest, ties-to-even.
    ///
    /// This models the x87 precision-control rounding applied to the
    /// superseded subtraction-based U64 conversion's FSUB result. It does
    /// not execute x87.
    fn round_integer_to_binary_precision(value: u128, precision: u32) -> u128 {
        assert!((1..=64).contains(&precision));

        if value == 0 {
            return 0;
        }

        let width = u128::BITS - value.leading_zeros();
        if width <= precision {
            return value;
        }

        let shift = width - precision;
        let retained = value >> shift;
        let discarded = value & ((1_u128 << shift) - 1);
        let halfway = 1_u128 << (shift - 1);

        let increment = discarded > halfway || (discarded == halfway && retained & 1 != 0);

        (retained + u128::from(increment)) << shift
    }

    /// The superseded high path computed fisttpq(x - 2^63). Under a 24- or
    /// 53-bit precision control the subtraction of an exactly representable
    /// extended input rounds to 2^63 — outside the signed range — and the
    /// conversion yields the integer-indefinite value. The native-payload
    /// path performs no arithmetic and is immune.
    #[test]
    fn subtraction_based_u64_conversion_has_precision_counterexample() {
        let exact_difference = (1_u128 << 63) - 1;

        for precision in [24, 53] {
            let rounded = round_integer_to_binary_precision(exact_difference, precision);

            assert_eq!(rounded, 1_u128 << 63);
            assert!(rounded > i64::MAX as u128);
        }

        assert_eq!(
            round_integer_to_binary_precision(exact_difference, 64),
            exact_difference
        );
    }

    #[test]
    fn precision_model_implements_ties_to_even() {
        assert_eq!(round_integer_to_binary_precision(17, 4), 16);
        assert_eq!(round_integer_to_binary_precision(19, 4), 20);
        assert_eq!(round_integer_to_binary_precision(21, 4), 20);
        assert_eq!(round_integer_to_binary_precision(23, 4), 24);
    }

    #[test]
    fn native_high_u64_payload_avoids_the_subtraction_counterexample() {
        let encoded = encode_high_u64(u64::MAX);

        assert_eq!(
            u16::from_le_bytes([encoded[8], encoded[9]]),
            U64_HIGH_X87_EXPONENT
        );

        let mut payload = [0_u8; 8];
        payload.copy_from_slice(&encoded[..8]);

        assert_eq!(u64::from_le_bytes(payload), u64::MAX);
    }

    #[test]
    fn sign_exponent_test_does_not_accept_negative_values() {
        let negative_exponent = U64_HIGH_X87_EXPONENT | 0x8000;
        assert_ne!(negative_exponent, U64_HIGH_X87_EXPONENT);

        for exponent in [
            0u16,
            U64_HIGH_X87_EXPONENT - 1,
            U64_HIGH_X87_EXPONENT + 1,
            0x7fff,
            0xffff,
        ] {
            assert_ne!(exponent, U64_HIGH_X87_EXPONENT);
        }
    }

    #[test]
    fn threshold_constant_is_exact() {
        assert_eq!(f64::from(f32::from_bits(0x5f00_0000)), TWO63);
        assert_eq!(f64::from(f32::from_bits(0x5f80_0000)), TWO64);
    }

    #[test]
    fn unsigned_conversion_boundary_cases() {
        let below_two63 = f64::from_bits(TWO63.to_bits() - 1);
        let above_two63 = f64::from_bits(TWO63.to_bits() + 1);
        let below_two64 = f64::from_bits(TWO64.to_bits() - 1);

        let cases = [
            (-0.75, 0),
            (-0.0, 0),
            (0.0, 0),
            (0.75, 0),
            (1.0, 1),
            (1.75, 1),
            (2_147_483_648.0, 2_147_483_648),
            (3_221_225_472.0, 3_221_225_472),
            (4_294_967_295.0, 4_294_967_295),
            (below_two63, (1u64 << 63) - 1024),
            (TWO63, 1u64 << 63),
            (above_two63, (1u64 << 63) + 2048),
            (13_835_058_055_282_163_712.0, 3u64 << 62),
            (below_two64, u64::MAX - 2047),
        ];

        for (value, expected) in cases {
            assert_eq!(split_f64_to_u64(value), expected, "value={value:?}");
        }
    }

    #[test]
    fn unsigned_conversion_random_finite_f64_samples() {
        let mut state = 0x6a09_e667_f3bc_c909u64;
        let mut checked = 0usize;

        for _ in 0..100_000 {
            let value = f64::from_bits(next_u64(&mut state));

            // Restrict the oracle to the defined C conversion domain.
            if value.is_finite() && value > -1.0 && value < TWO64 {
                assert_eq!(split_f64_to_u64(value), value as u64);
                checked += 1;
            }
        }

        assert!(checked > 1000);
    }

    #[test]
    fn signedness_change_regressions() {
        // Semantic examples for the re-canonicalization rules.
        assert_eq!(i32::from(255u8 as i8), -1);
        assert_eq!(i32::from(65535u16 as i16), -1);

        assert_eq!(u32::from((-1i8) as u16), 65535);
        assert_eq!(u32::from((-128i8) as u16), 65408);

        assert_eq!(u32::from((-1i16) as u8), 255);
        assert_eq!(i32::from(128u8 as i16), 128);
    }
}
