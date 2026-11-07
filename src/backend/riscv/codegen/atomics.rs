//! RISC-V sub-word atomic operations, atomic load/store/RMW/CAS, fence,
//! and software implementations of CLZ/CTZ/BSWAP/POPCOUNT builtins.
//!
//! RISC-V only provides word (32-bit) and doubleword (64-bit) atomic instructions
//! (AMO and LR/SC). For sub-word types (I8/U8/I16/U16), this module implements
//! atomic RMW and CAS operations using word-aligned LR.W/SC.W loops with bit
//! masking. It also provides software, branch-free implementations of count
//! leading zeros, count trailing zeros, byte swap, and population count for
//! targets that lack the Zbb extension.

use super::emit::RiscvCodegen;
use super::emit::callee_saved_name;
use crate::backend::state::CodegenState;
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, IrConst, Operand, Value};

impl RiscvCodegen {
    /// Get the AMO ordering suffix.
    pub(super) fn amo_ordering(ordering: AtomicOrdering) -> &'static str {
        match ordering {
            AtomicOrdering::Relaxed => "",
            AtomicOrdering::Acquire => ".aq",
            AtomicOrdering::Release => ".rl",
            AtomicOrdering::AcqRel => ".aqrl",
            AtomicOrdering::SeqCst => ".aqrl",
        }
    }

    /// Get the AMO width suffix for word/doubleword operations.
    /// Sub-word types (I8/U8/I16/U16) should use the sub-word atomic helpers instead.
    pub(super) fn amo_width_suffix(ty: IrType) -> &'static str {
        // The suffix IS the access width: a 4-byte `float` takes the word
        // forms (it used to fall into `d`, an 8-byte AMO on a 4-byte object).
        match ty {
            IrType::I32 | IrType::U32 | IrType::F32 => "w",
            _ => "d",
        }
    }

    /// Sign-extend result for sub-word types after atomic ops.
    pub(super) fn sign_extend_riscv(state: &mut CodegenState, ty: IrType) {
        match ty {
            IrType::I8 => {
                state.emit("    slli t0, t0, 56");
                state.emit("    srai t0, t0, 56");
            }
            IrType::U8 => {
                state.emit("    andi t0, t0, 0xff");
            }
            IrType::I16 => {
                state.emit("    slli t0, t0, 48");
                state.emit("    srai t0, t0, 48");
            }
            IrType::U16 => {
                state.emit("    slli t0, t0, 48");
                state.emit("    srli t0, t0, 48");
            }
            IrType::I32 => {
                state.emit("    sext.w t0, t0");
            }
            IrType::U32 => {
                // AMO.W/LR.W sign-extend their result on RV64; unsigned IR
                // values require explicit zero-extension.
                state.emit("    slli t0, t0, 32");
                state.emit("    srli t0, t0, 32");
            }
            _ => {}
        }
    }

    /// Check if a type requires sub-word atomic handling on RISC-V.
    /// RISC-V only has word (32-bit) and doubleword (64-bit) atomic instructions.
    pub(super) fn is_subword_type(ty: IrType) -> bool {
        matches!(ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16)
    }

    /// Get the bit width of a sub-word type.
    pub(super) fn subword_bits(ty: IrType) -> u32 {
        match ty {
            IrType::I8 | IrType::U8 => 8,
            IrType::I16 | IrType::U16 => 16,
            _ => unreachable!("subword_bits called with non-subword type: {:?}", ty),
        }
    }

    /// Emit sub-word atomic RMW using word-aligned LR.W/SC.W with bit masking.
    ///
    /// On entry: t1 = original ptr, t2 = value to apply
    /// On exit: t0 = old sub-word value (not sign-extended yet)
    ///
    /// Strategy: Align the address to a word boundary, compute shift and mask,
    /// then use LR.W/SC.W loop operating on the containing word while only
    /// modifying the sub-word field.
    ///
    /// Register usage (all caller-saved):
    ///   a2 = word-aligned address
    ///   a3 = bit shift amount
    ///   a4 = shifted mask for sub-word field
    ///   a5 = inverted mask (~mask)
    ///   t0 = loaded word (old value)
    ///   t2 = shifted value operand
    ///   t3 = temporary, t4 = new word to store, t5 = SC result flag
    pub(super) fn emit_subword_atomic_rmw(
        &mut self,
        op: AtomicRmwOp,
        ty: IrType,
        lr_order: &str,
        sc_order: &str,
    ) {
        let bits = Self::subword_bits(ty);
        let loop_label = self.state.fresh_label("sw_rmw_loop");
        let done_label = self.state.fresh_label("sw_rmw_done");

        // a2 = word-aligned address (ptr & ~3)
        self.state.emit("    andi a2, t1, -4");
        // a3 = byte offset within word: (ptr & 3)
        self.state.emit("    andi a3, t1, 3");
        // a3 = bit shift = byte_offset * 8
        self.state.emit("    slli a3, a3, 3");
        // a4 = mask for the sub-word field (e.g., 0xFF or 0xFFFF shifted into position)
        if bits == 8 {
            self.state.emit("    li a4, 0xff");
        } else {
            // 16-bit: can't use andi with 0xffff, load it
            self.state.emit("    lui a4, 16"); // a4 = 0x10000
            self.state.emit("    addiw a4, a4, -1"); // a4 = 0xFFFF
        }
        self.state.emit("    sllw a4, a4, a3"); // a4 = mask << shift
        // a5 = ~mask (inverted mask for clearing the field)
        self.state.emit("    not a5, a4");
        // Shift the value into position: t2 = (val & field_mask) << shift
        if bits == 8 {
            self.state.emit("    andi t2, t2, 0xff");
        } else {
            self.state.emit("    slli t2, t2, 48");
            self.state.emit("    srli t2, t2, 48");
        }
        self.state.emit("    sllw t2, t2, a3"); // t2 = val << shift

        // LR/SC loop
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state
            .emit_fmt(format_args!("    lr.w{} t0, (a2)", lr_order));

        match op {
            AtomicRmwOp::Xchg | AtomicRmwOp::TestAndSet => {
                // new_word = (old_word & ~mask) | (new_val & mask)
                if matches!(op, AtomicRmwOp::TestAndSet) {
                    // Override: set the byte to 1
                    self.state.emit("    li t3, 1");
                    self.state.emit("    sllw t3, t3, a3");
                } else {
                    self.state.emit("    mv t3, t2");
                }
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
            AtomicRmwOp::Add => {
                // Extract old sub-word, add val, insert result
                self.state.emit("    and t3, t0, a4"); // t3 = old field (shifted)
                self.state.emit("    add t3, t3, t2"); // t3 = old + val (shifted)
                self.state.emit("    and t3, t3, a4"); // mask to field width
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
            AtomicRmwOp::Sub => {
                // Extract old sub-word, subtract val, insert result
                self.state.emit("    and t3, t0, a4"); // t3 = old field (shifted)
                self.state.emit("    sub t3, t3, t2"); // t3 = old - val (shifted)
                self.state.emit("    and t3, t3, a4"); // mask to field width
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
            AtomicRmwOp::And => {
                // new_field = old_field & val_field
                // For AND: bits outside the field should remain unchanged.
                // new_word = old_word & (val_shifted | ~mask)
                self.state.emit("    or t3, t2, a5"); // val_shifted | ~mask
                self.state.emit("    and t4, t0, t3"); // old & (val | ~mask)
            }
            AtomicRmwOp::Or => {
                // new_word = old_word | (val_shifted & mask)
                self.state.emit("    and t3, t2, a4"); // val & mask (already masked, but safe)
                self.state.emit("    or t4, t0, t3");
            }
            AtomicRmwOp::Xor => {
                // new_word = old_word ^ (val_shifted & mask)
                self.state.emit("    and t3, t2, a4");
                self.state.emit("    xor t4, t0, t3");
            }
            AtomicRmwOp::Nand => {
                // new_field = ~(old_field & val_field)
                self.state.emit("    and t3, t0, a4"); // old field
                self.state.emit("    and t3, t3, t2"); // old & val (shifted)
                self.state.emit("    not t3, t3"); // ~(old & val) - full word invert
                self.state.emit("    and t3, t3, a4"); // mask to field
                self.state.emit("    and t4, t0, a5"); // clear old field
                self.state.emit("    or t4, t4, t3"); // insert new value
            }
        }

        // SC: rd (t5) must differ from rs2 (t4) per RISC-V spec
        self.state
            .emit_fmt(format_args!("    sc.w{} t5, t4, (a2)", sc_order));
        self.state
            .emit_fmt(format_args!("    bnez t5, {}", loop_label));
        self.state.emit_fmt(format_args!("{}:", done_label));
        // Extract the old sub-word value: t0 = (old_word >> shift) & field_mask
        self.state.emit("    srlw t0, t0, a3");
        if bits == 8 {
            self.state.emit("    andi t0, t0, 0xff");
        } else {
            self.state.emit("    slli t0, t0, 48");
            self.state.emit("    srli t0, t0, 48");
        }
    }

    /// Emit sub-word atomic CAS using word-aligned LR.W/SC.W with bit masking.
    ///
    /// On entry: t1 = ptr, t2 = expected, t3 = desired
    /// On exit: t0 = old sub-word value (for !returns_bool) or success flag
    ///
    /// Register usage (all caller-saved):
    ///   a2 = word-aligned address
    ///   a3 = bit shift amount
    ///   a4 = shifted mask
    ///   a5 = inverted mask (~mask)
    ///   t0 = loaded word
    ///   t2 = shifted expected, t3 = shifted desired
    ///   t4 = new word to store, t5 = SC result flag
    pub(super) fn emit_subword_atomic_cmpxchg(
        &mut self,
        ty: IrType,
        lr_order: &str,
        sc_order: &str,
        returns_bool: bool,
    ) {
        let bits = Self::subword_bits(ty);
        let loop_label = self.state.fresh_label("sw_cas_loop");
        let fail_label = self.state.fresh_label("sw_cas_fail");
        let done_label = self.state.fresh_label("sw_cas_done");

        // a2 = word-aligned address
        self.state.emit("    andi a2, t1, -4");
        // a3 = bit shift
        self.state.emit("    andi a3, t1, 3");
        self.state.emit("    slli a3, a3, 3");
        // a4 = mask
        if bits == 8 {
            self.state.emit("    li a4, 0xff");
        } else {
            self.state.emit("    lui a4, 16");
            self.state.emit("    addiw a4, a4, -1");
        }
        self.state.emit("    sllw a4, a4, a3");
        // a5 = ~mask
        self.state.emit("    not a5, a4");
        // Mask and shift expected and desired
        if bits == 8 {
            self.state.emit("    andi t2, t2, 0xff");
        } else {
            self.state.emit("    slli t2, t2, 48");
            self.state.emit("    srli t2, t2, 48");
        }
        self.state.emit("    sllw t2, t2, a3"); // t2 = expected << shift
        if bits == 8 {
            self.state.emit("    andi t3, t3, 0xff");
        } else {
            self.state.emit("    slli t3, t3, 48");
            self.state.emit("    srli t3, t3, 48");
        }
        self.state.emit("    sllw t3, t3, a3"); // t3 = desired << shift

        // LR/SC loop
        self.state.emit_fmt(format_args!("{}:", loop_label));
        self.state
            .emit_fmt(format_args!("    lr.w{} t0, (a2)", lr_order));
        // Compare only the sub-word field
        self.state.emit("    and t4, t0, a4"); // t4 = current field
        self.state
            .emit_fmt(format_args!("    bne t4, t2, {}", fail_label));
        // Build new word: (old & ~mask) | desired_shifted
        self.state.emit("    and t4, t0, a5");
        self.state.emit("    or t4, t4, t3");
        // SC: rd (t5) must differ from rs2 (t4) per RISC-V spec
        self.state
            .emit_fmt(format_args!("    sc.w{} t5, t4, (a2)", sc_order));
        self.state
            .emit_fmt(format_args!("    bnez t5, {}", loop_label));
        // Success
        if returns_bool {
            self.state.emit("    li t0, 1");
        } else {
            // Extract old sub-word value
            self.state.emit("    srlw t0, t0, a3");
            if bits == 8 {
                self.state.emit("    andi t0, t0, 0xff");
            } else {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
        }
        self.state.emit_fmt(format_args!("    j {}", done_label));
        self.state.emit_fmt(format_args!("{}:", fail_label));
        if returns_bool {
            self.state.emit("    li t0, 0");
        } else {
            // Extract old sub-word value from loaded word
            self.state.emit("    srlw t0, t0, a3");
            if bits == 8 {
                self.state.emit("    andi t0, t0, 0xff");
            } else {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48");
            }
        }
        self.state.emit_fmt(format_args!("{}:", done_label));
    }

    /// Software CLZ (count leading zeros). Input in t0, result in t0.
    ///
    /// Branch-free: `clz` is `bits - bitlen`, and `bitlen` is the population
    /// count of the operand *smeared* downward (`x |= x >> 1`, `x >> 2`, ...),
    /// which turns on every bit below the highest set bit.  `x == 0` smears to
    /// zero, so a zero operand falls out of the count as `bits` with no
    /// special case -- the same "returns the width for zero" contract the scan
    /// had.  Two instructions per smearing stage, `log2(bits)` stages, then the
    /// branch-free population count; the per-bit scan this replaces ran up to
    /// `bits` trips through two unpredictable branches, so the new form is
    /// constant-cost *and* constant-time.
    pub(super) fn emit_clz(&mut self, ty: IrType) {
        let bits: u64 = match ty {
            IrType::I32 | IrType::U32 => 32,
            _ => 64,
        };
        if bits == 32 {
            // Zero-extend first: a sign-extended operand would smear its sign
            // bits down into the count and report zero leading zeros.
            self.state.emit("    slli t0, t0, 32");
            self.state.emit("    srli t0, t0, 32");
        }
        let mut shift = 1;
        while shift < bits {
            self.state
                .emit_fmt(format_args!("    srli t1, t0, {shift}"));
            self.state.emit("    or t0, t0, t1");
            shift *= 2;
        }
        // The complement of the smear has exactly `clz` low ones.  The
        // population count masks the operand to the type, which drops the 64
        // ones the complement left above a 32-bit operand.
        self.state.emit("    not t0, t0");
        self.emit_popcount(ty);
    }

    /// Software CTZ (count trailing zeros). Input in t0, result in t0.
    ///
    /// Branch-free: `(x - 1) & ~x` keeps exactly the bits below the lowest set
    /// bit, so its population count is the trailing-zero count; for `x == 0`
    /// both halves are all ones and the count is the full width, which is the
    /// contract the scan had.  Three instructions plus the count instead of up
    /// to `bits` trips through two unpredictable branches.
    pub(super) fn emit_ctz(&mut self, ty: IrType) {
        self.state.emit("    addi t1, t0, -1");
        self.state.emit("    not t2, t0");
        self.state.emit("    and t0, t1, t2");
        self.emit_popcount(ty);
    }

    /// Software BSWAP (byte swap). Input in t0, result in t0.
    pub(super) fn emit_bswap(&mut self, ty: IrType) {
        match ty {
            IrType::I16 | IrType::U16 => {
                // Swap bytes of a 16-bit value
                // t1 = (t0 >> 8) & 0xFF, t2 = (t0 & 0xFF) << 8
                self.state.emit("    andi t1, t0, 0xff");
                self.state.emit("    slli t1, t1, 8");
                self.state.emit("    srli t0, t0, 8");
                self.state.emit("    andi t0, t0, 0xff");
                self.state.emit("    or t0, t0, t1");
            }
            IrType::I32 | IrType::U32 => {
                // Swap 4 bytes: ABCD -> DCBA
                self.state.emit("    mv t1, t0");
                // Byte 0 -> byte 3
                self.state.emit("    andi t2, t1, 0xff");
                self.state.emit("    slli t0, t2, 24");
                // Byte 1 -> byte 2
                self.state.emit("    srli t2, t1, 8");
                self.state.emit("    andi t2, t2, 0xff");
                self.state.emit("    slli t2, t2, 16");
                self.state.emit("    or t0, t0, t2");
                // Byte 2 -> byte 1
                self.state.emit("    srli t2, t1, 16");
                self.state.emit("    andi t2, t2, 0xff");
                self.state.emit("    slli t2, t2, 8");
                self.state.emit("    or t0, t0, t2");
                // Byte 3 -> byte 0
                self.state.emit("    srli t2, t1, 24");
                self.state.emit("    andi t2, t2, 0xff");
                self.state.emit("    or t0, t0, t2");
                // Zero-extend to 32 bits (clear upper 32 bits)
                self.state.emit("    slli t0, t0, 32");
                self.state.emit("    srli t0, t0, 32");
            }
            _ => {
                // 64-bit byte swap: reverse all 8 bytes
                self.state.emit("    mv t1, t0");
                self.state.emit("    li t0, 0");
                // Use a shift-based approach: extract each byte and place it
                for i in 0..8u64 {
                    let src_shift = i * 8;
                    let dst_shift = (7 - i) * 8;
                    self.state
                        .emit_fmt(format_args!("    srli t2, t1, {}", src_shift));
                    self.state.emit("    andi t2, t2, 0xff");
                    if dst_shift > 0 {
                        self.state
                            .emit_fmt(format_args!("    slli t2, t2, {}", dst_shift));
                    }
                    self.state.emit("    or t0, t0, t2");
                }
            }
        }
    }

    /// POPCOUNT: the number of set bits.  Input in t0, result in t0.
    ///
    /// The previous lowering was Kernighan's bit-clearing loop -- five
    /// instructions *per set bit* behind a data-dependent branch, so up to 64
    /// iterations and an unpredictable one per trip.  That is the defect the
    /// x86 backend shed when it moved to a branch-free sequence, and it costs
    /// more here because nothing in the loop lets the machine see the trip count
    /// ahead of time.
    ///
    /// This is the SWAR "count within lanes, then add the lanes" form: pairwise
    /// sums fit in two bits, nibble sums in three and byte sums in four, so the
    /// three mask/add stages never carry between lanes.  Past the byte stage
    /// every lane holds a value of at most 8 and the total is at most 64, so the
    /// remaining stages are plain shift/add pairs with no masks -- no `mul`, and
    /// therefore no dependence on the M extension either.
    pub(super) fn emit_popcount(&mut self, ty: IrType) {
        // A byte or a halfword counts inside its own lanes, so every mask below
        // fits the 12-bit immediate field of `andi`: three instructions fewer
        // than the 32-bit form even before the constant loads, and no scratch
        // register for the masks.  The operand is masked to its type first --
        // the lanes are read out of the whole register, so a sign extension or a
        // stale high half must not reach the count.
        match ty {
            IrType::I8 | IrType::U8 => {
                self.state.emit("    andi t0, t0, 0xff");
                self.state.emit("    srli t1, t0, 1");
                self.state.emit("    andi t1, t1, 0x55");
                self.state.emit("    sub t1, t0, t1"); // pair counts
                self.state.emit("    srli t3, t1, 2");
                self.state.emit("    andi t3, t3, 0x33");
                self.state.emit("    andi t2, t1, 0x33");
                self.state.emit("    add t1, t2, t3"); // nibble counts
                self.state.emit("    srli t3, t1, 4");
                self.state.emit("    add t1, t1, t3");
                self.state.emit("    andi t0, t1, 0x0f"); // byte count
                return;
            }
            IrType::I16 | IrType::U16 => {
                self.state.emit("    slli t0, t0, 48");
                self.state.emit("    srli t0, t0, 48"); // zero the halfword
                self.state.emit("    srli t1, t0, 1");
                self.state.emit("    li t2, 0x5555");
                self.state.emit("    and t1, t1, t2");
                self.state.emit("    sub t1, t0, t1");
                self.state.emit("    li t2, 0x3333");
                self.state.emit("    srli t3, t1, 2");
                self.state.emit("    and t3, t3, t2");
                self.state.emit("    and t1, t1, t2");
                self.state.emit("    add t1, t1, t3"); // nibble counts
                self.state.emit("    srli t3, t1, 4");
                self.state.emit("    add t1, t1, t3"); // byte counts in bytes 0 and 1
                self.state.emit("    andi t3, t1, 0x0f");
                self.state.emit("    srli t1, t1, 8");
                self.state.emit("    andi t1, t1, 0x0f");
                self.state.emit("    add t0, t3, t1");
                return;
            }
            _ => {}
        }
        let wide = ty == IrType::I64 || ty == IrType::U64;
        let (pairs, nibbles, bytes) = if wide {
            (
                "0x5555555555555555",
                "0x3333333333333333",
                "0x0f0f0f0f0f0f0f0f",
            )
        } else {
            // 32-bit operands: a sign-extended value must not count its sign
            // bits, so the low word is zero-extended first.
            self.state.emit("    slli t0, t0, 32");
            self.state.emit("    srli t0, t0, 32");
            ("0x55555555", "0x33333333", "0x0f0f0f0f")
        };
        self.state.emit("    srli t1, t0, 1");
        self.state.emit_fmt(format_args!("    li t2, {pairs}"));
        self.state.emit("    and t1, t1, t2");
        self.state.emit("    sub t1, t0, t1"); // t1 = pair counts
        self.state.emit_fmt(format_args!("    li t2, {nibbles}"));
        self.state.emit("    srli t3, t1, 2");
        self.state.emit("    and t3, t3, t2");
        self.state.emit("    and t1, t1, t2");
        self.state.emit("    add t1, t1, t3"); // nibble counts
        self.state.emit("    srli t3, t1, 4");
        self.state.emit("    add t1, t1, t3");
        self.state.emit_fmt(format_args!("    li t2, {bytes}"));
        self.state.emit("    and t1, t1, t2"); // byte counts
        let stages: &[u32] = if wide { &[8, 16, 32] } else { &[8, 16] };
        for &shift in stages {
            self.state
                .emit_fmt(format_args!("    srli t3, t1, {shift}"));
            self.state.emit("    add t1, t1, t3");
        }
        self.state.emit("    andi t0, t1, 0x7f");
    }

    /// PARITY: the XOR of every bit, as 0 or 1.  Input in t0, result in t0.
    ///
    /// Parity is the count modulo two, so the SWAR count above is overkill: a
    /// chain of XOR folds (`x ^ x>>k` preserves parity) squeezes the operand
    /// down to a nibble, and bit *n* of 0x6996 is exactly the parity of *n*, so
    /// one variable shift reads the answer out of the constant.  That is eleven
    /// instructions on 32 bits where the `popcount & 1` form was twenty-three
    /// (and, before the count was made branch-free, up to sixty-four trips).
    pub(super) fn emit_parity(&mut self, ty: IrType) {
        // A byte or halfword needs one fold stage per width it has: shift by
        // half the type, then 4, 2 and 1, and the low bit is the parity.  The
        // nibble table below is a win only when the operand is wider, because
        // it exists to skip the tail of a 32- or 64-bit reduction.
        let narrow = match ty {
            IrType::I8 | IrType::U8 => Some(4),
            IrType::I16 | IrType::U16 => Some(8),
            _ => None,
        };
        if let Some(first) = narrow {
            // Mask to the type first: a halfword operand must not fold bits of
            // the word above it into the answer.
            match ty {
                IrType::I8 | IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                _ => {
                    self.state.emit("    slli t0, t0, 48");
                    self.state.emit("    srli t0, t0, 48");
                }
            }
            self.state
                .emit_fmt(format_args!("    srli t1, t0, {first}"));
            self.state.emit("    xor t1, t0, t1");
            // The first fold covered `first` bits; the rest halve what is left.
            let rest: &[u32] = if first == 8 { &[4, 2, 1] } else { &[2, 1] };
            for &shift in rest {
                self.state
                    .emit_fmt(format_args!("    srli t2, t1, {shift}"));
                self.state.emit("    xor t1, t1, t2");
            }
            self.state.emit("    andi t0, t1, 1");
            return;
        }
        let wide = ty == IrType::I64 || ty == IrType::U64;
        if wide {
            self.state.emit("    srli t1, t0, 32");
            self.state.emit("    xor t1, t0, t1");
        } else {
            self.state.emit("    srli t1, t0, 16");
            self.state.emit("    xor t1, t0, t1");
        }
        if wide {
            // 64 -> 32 above, 32 -> 16 here; the 32-bit operand started at the
            // 16-bit fold, so this pair is only needed for the wide case.
            self.state.emit("    srli t2, t1, 16");
            self.state.emit("    xor t1, t1, t2");
        }
        self.state.emit("    srli t2, t1, 8");
        self.state.emit("    xor t1, t1, t2");
        self.state.emit("    srli t2, t1, 4");
        self.state.emit("    xor t1, t1, t2");
        self.state.emit("    andi t1, t1, 0xf");
        self.state.emit("    li t2, 0x6996");
        self.state.emit("    srl t2, t2, t1");
        self.state.emit("    andi t0, t2, 1");
    }

    // ---- Trait-level atomic operations (delegated from ArchCodegen) ----
    //
    // Register contract (shared with the allocator's atomic-pointer
    // eligibility, `regalloc.rs`): the word/doubleword sequences write t0
    // (old value), t2 (a negated, sign-extended or constant operand), t3
    // (Nand's new value) and t4 (store-conditional status); the sub-word
    // loops also write t1-t5 and a2-a5.  An operand's home (a0-a7, s1-s11;
    // never a t-register) is read in place, and a function containing a
    // sub-word RMW/CAS never opens the a-register pool
    // (`riscv_emitter_safe_ir`), so nothing lives in a2-a5 across one.
    // LR/SC orderings follow the RVWMO mapping of the unprivileged ISA
    // (Table A.6): `lr.aq` for acquire, `lr.aqrl` for seq_cst, `sc.rl` for
    // release/acq_rel/seq_cst -- never the `lr.rl`/`sc.aq` forms the spec
    // says give no stronger ordering than plain ones.  16-byte operations
    // call libatomic's `__atomic_*_16`, as GCC 16.2 and Clang 23.1.5 do.

    /// LR suffix for a loop of ordering `ordering` (Table A.6).
    fn lr_ordering(ordering: AtomicOrdering) -> &'static str {
        match ordering {
            AtomicOrdering::Relaxed | AtomicOrdering::Release => "",
            AtomicOrdering::Acquire | AtomicOrdering::AcqRel => ".aq",
            AtomicOrdering::SeqCst => ".aqrl",
        }
    }

    /// SC suffix for a loop of ordering `ordering` (Table A.6).
    fn sc_ordering(ordering: AtomicOrdering) -> &'static str {
        match ordering {
            AtomicOrdering::Relaxed | AtomicOrdering::Acquire => "",
            AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst => ".rl",
        }
    }

    /// The home register of `op` (a0-a7 or s1-s11), if it has one.
    fn atomic_home(&self, op: &Operand) -> Option<&'static str> {
        let Operand::Value(v) = op else {
            return None;
        };
        if self.state.is_alloca(v.0) {
            return None;
        }
        self.reg_assignments
            .get(&v.0)
            .map(|&reg| callee_saved_name(reg))
    }

    /// A constant whose bits are all zero (never `-0.0`): `zero` supplies it.
    fn atomic_zero_operand(op: &Operand) -> bool {
        match op {
            Operand::Const(c) => match c {
                IrConst::I8(v) => *v == 0,
                IrConst::I16(v) => *v == 0,
                IrConst::I32(v) => *v == 0,
                IrConst::I64(v) => *v == 0,
                IrConst::F32(v) => v.to_bits() == 0,
                IrConst::F64(v) => v.to_bits() == 0,
                IrConst::Zero => true,
                _ => false,
            },
            Operand::Value(_) => false,
        }
    }

    /// The register an atomic sequence reads `op` from: its home, `zero`
    /// for an all-zero constant when `allow_zero`, else `scratch` loaded
    /// through t0.  Operands are fetched one at a time, so one staged
    /// earlier must sit in a register other than t0.
    fn atomic_operand_reg(
        &mut self,
        op: &Operand,
        scratch: &'static str,
        allow_zero: bool,
    ) -> &'static str {
        if allow_zero && Self::atomic_zero_operand(op) {
            return "zero";
        }
        if let Some(home) = self.atomic_home(op) {
            return home;
        }
        if let Some(value) = Self::atomic_int_const(op) {
            // Straight into the scratch: through t0 the constant left a
            // dead `li t0, c` that the AMO or LR overwrites.
            if scratch == "t0" {
                self.state.reg_cache.invalidate_acc();
            }
            self.state
                .emit_fmt(format_args!("    li {scratch}, {value}"));
            return scratch;
        }
        self.operand_to_t0(op);
        if scratch != "t0" {
            self.state.emit_fmt(format_args!("    mv {scratch}, t0"));
        }
        scratch
    }

    /// Copy `op` into `scratch` (the sub-word loops rewrite their inputs).
    fn atomic_stage(&mut self, op: &Operand, scratch: &'static str) {
        let reg = self.atomic_operand_reg(op, scratch, true);
        if reg != scratch {
            self.state.emit_fmt(format_args!("    mv {scratch}, {reg}"));
        }
    }

    pub(super) fn emit_atomic_rmw_impl(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        if matches!(ty, IrType::I128 | IrType::U128) {
            self.emit_atomic_rmw_128(dest, op, ptr, val, ordering);
            return;
        }
        if Self::is_subword_type(ty) {
            self.atomic_stage(val, "t2");
            self.atomic_stage(ptr, "t1");
            self.emit_subword_atomic_rmw(
                op,
                ty,
                Self::lr_ordering(ordering),
                Self::sc_ordering(ordering),
            );
            Self::sign_extend_riscv(&mut self.state, ty);
            self.store_t0_to(dest);
            return;
        }
        let aq_rl = Self::amo_ordering(ordering);
        let suffix = Self::amo_width_suffix(ty);
        // The AMO's rs2: the value's home in place, `zero`, or t2 (a
        // negated subtrahend -- a constant one folded -- or a constant).
        let val_reg = match op {
            AtomicRmwOp::TestAndSet => {
                self.state.emit("    li t2, 1");
                "t2"
            }
            AtomicRmwOp::Sub => {
                if let Some(c) = Self::atomic_int_const(val) {
                    self.state
                        .emit_fmt(format_args!("    li t2, {}", c.wrapping_neg()));
                } else {
                    let reg = self.atomic_operand_reg(val, "t2", true);
                    self.state.emit_fmt(format_args!("    neg t2, {reg}"));
                }
                "t2"
            }
            _ => self.atomic_operand_reg(val, "t2", true),
        };
        let ptr_reg = self.atomic_operand_reg(ptr, "t1", false);
        let amo = match op {
            AtomicRmwOp::Add | AtomicRmwOp::Sub => Some("amoadd"),
            AtomicRmwOp::And => Some("amoand"),
            AtomicRmwOp::Or => Some("amoor"),
            AtomicRmwOp::Xor => Some("amoxor"),
            AtomicRmwOp::Xchg | AtomicRmwOp::TestAndSet => Some("amoswap"),
            AtomicRmwOp::Nand => None,
        };
        if let Some(amo) = amo {
            // A result that needs no extension (AMO.W sign-extends, which is
            // the canonical `int`; doubleword results are whole) goes
            // straight to its home: an AMO's rd may name its own address or
            // data register, as Clang's `amoadd.d.aqrl a0, a1, (a0)` does.
            let direct = if matches!(ty, IrType::I32 | IrType::I64 | IrType::U64 | IrType::Ptr) {
                self.atomic_home(&Operand::Value(*dest))
            } else {
                None
            };
            let rd = direct.unwrap_or("t0");
            self.state.emit_fmt(format_args!(
                "    {amo}.{suffix}{aq_rl} {rd}, {val_reg}, ({ptr_reg})"
            ));
            if direct.is_some() {
                self.state.reg_cache.invalidate_acc();
                return;
            }
        } else {
            let lr = Self::lr_ordering(ordering);
            let sc = Self::sc_ordering(ordering);
            let loop_label = self.state.fresh_label("atomic_nand");
            self.state.emit_fmt(format_args!("{loop_label}:"));
            self.state
                .emit_fmt(format_args!("    lr.{suffix}{lr} t0, ({ptr_reg})"));
            self.state
                .emit_fmt(format_args!("    and t3, t0, {val_reg}"));
            self.state.emit("    not t3, t3");
            self.state
                .emit_fmt(format_args!("    sc.{suffix}{sc} t4, t3, ({ptr_reg})"));
            self.state
                .emit_fmt(format_args!("    bnez t4, {loop_label}"));
        }
        // AMO.W and LR.W sign-extend their word on RV64: an `int` result is
        // already canonical.
        if ty != IrType::I32 {
            Self::sign_extend_riscv(&mut self.state, ty);
        }
        self.store_t0_to(dest);
    }

    /// An integer constant operand's value.
    fn atomic_int_const(op: &Operand) -> Option<i64> {
        match op {
            Operand::Const(IrConst::I8(v)) => Some(i64::from(*v)),
            Operand::Const(IrConst::I16(v)) => Some(i64::from(*v)),
            Operand::Const(IrConst::I32(v)) => Some(i64::from(*v)),
            Operand::Const(IrConst::I64(v)) => Some(*v),
            Operand::Const(IrConst::Zero) => Some(0),
            _ => None,
        }
    }

    pub(super) fn emit_atomic_cmpxchg_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        if matches!(ty, IrType::I128 | IrType::U128) {
            self.emit_atomic_cmpxchg_128(
                dest,
                ptr,
                expected,
                desired,
                ordering,
                failure_ordering,
                returns_bool,
            );
            return;
        }
        let lr = Self::lr_ordering(ordering);
        let sc = Self::sc_ordering(ordering);
        if Self::is_subword_type(ty) {
            self.atomic_stage(desired, "t3");
            self.atomic_stage(expected, "t2");
            self.atomic_stage(ptr, "t1");
            self.emit_subword_atomic_cmpxchg(ty, lr, sc, returns_bool);
            if !returns_bool {
                Self::sign_extend_riscv(&mut self.state, ty);
            }
            self.store_t0_to(dest);
            return;
        }
        let suffix = Self::amo_width_suffix(ty);
        let desired_reg = self.atomic_operand_reg(desired, "t3", true);
        // LR.W sign-extends on RV64: compare the expected value in the same
        // representation, including unsigned values >= 2^31.
        let expected_reg = if matches!(ty, IrType::I32 | IrType::U32 | IrType::F32) {
            if Self::atomic_zero_operand(expected) {
                "zero"
            } else {
                let reg = self.atomic_operand_reg(expected, "t2", true);
                self.state.emit_fmt(format_args!("    sext.w t2, {reg}"));
                "t2"
            }
        } else {
            self.atomic_operand_reg(expected, "t2", true)
        };
        let ptr_reg = self.atomic_operand_reg(ptr, "t1", false);
        let loop_label = self.state.fresh_label("cas_loop");
        let fail_label = self.state.fresh_label("cas_fail");
        self.state.emit_fmt(format_args!("{loop_label}:"));
        self.state
            .emit_fmt(format_args!("    lr.{suffix}{lr} t0, ({ptr_reg})"));
        self.state
            .emit_fmt(format_args!("    bne t0, {expected_reg}, {fail_label}"));
        self.state.emit_fmt(format_args!(
            "    sc.{suffix}{sc} t4, {desired_reg}, ({ptr_reg})"
        ));
        self.state
            .emit_fmt(format_args!("    bnez t4, {loop_label}"));
        self.state.emit_fmt(format_args!("{fail_label}:"));
        if returns_bool {
            // Both paths meet here: t0 equals the expected value exactly
            // when the exchange happened.
            self.state
                .emit_fmt(format_args!("    sub t0, t0, {expected_reg}"));
            self.state.emit("    seqz t0, t0");
        } else if ty != IrType::I32 {
            // LR.W already sign-extends an `int`.
            Self::sign_extend_riscv(&mut self.state, ty);
        }
        self.store_t0_to(dest);
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        if matches!(ty, IrType::I128 | IrType::U128) {
            self.emit_atomic_load_128(dest, ptr, ordering);
            return;
        }
        let ptr_reg = self.atomic_operand_reg(ptr, "t0", false);
        // Naturally aligned loads are single-copy atomic; LR would only
        // create a reservation.  The fences supply the ordering (Table A.6).
        if matches!(ordering, AtomicOrdering::SeqCst) {
            self.state.emit("    fence rw, rw");
        }
        let load = match ty {
            IrType::I8 => "lb",
            IrType::U8 => "lbu",
            IrType::I16 => "lh",
            IrType::U16 => "lhu",
            IrType::I32 => "lw",
            IrType::U32 | IrType::F32 => "lwu",
            _ => "ld",
        };
        self.state
            .emit_fmt(format_args!("    {load} t0, 0({ptr_reg})"));
        if matches!(
            ordering,
            AtomicOrdering::Acquire | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
        ) {
            self.state.emit("    fence r, rw");
        }
        self.store_t0_to(dest);
    }

    pub(super) fn emit_atomic_store_impl(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        if matches!(ty, IrType::I128 | IrType::U128) {
            self.emit_atomic_store_128(ptr, val, ordering);
            return;
        }
        let val_reg = self.atomic_operand_reg(val, "t1", true);
        let ptr_reg = self.atomic_operand_reg(ptr, "t0", false);
        // Naturally aligned stores are atomic: the RVWMO mapping, not an
        // AMOSWAP whose old value is discarded.  The trailing fence of a
        // seq_cst store is Clang 23.1.5's default, compatible with code
        // built for either fence convention.
        if matches!(
            ordering,
            AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
        ) {
            self.state.emit("    fence rw, w");
        }
        let store = match ty {
            IrType::I8 | IrType::U8 => "sb",
            IrType::I16 | IrType::U16 => "sh",
            IrType::I32 | IrType::U32 | IrType::F32 => "sw",
            _ => "sd",
        };
        self.state
            .emit_fmt(format_args!("    {store} {val_reg}, 0({ptr_reg})"));
        if matches!(ordering, AtomicOrdering::SeqCst) {
            self.state.emit("    fence rw, rw");
        }
    }

    // ---- 128-bit: libatomic calls ----
    //
    // A 16-byte atomic is a call point for the allocator
    // (`liveness::instruction_is_call_point`) and keeps its function out of
    // the a-register pool, the frameless and the fp-less frames
    // (`riscv_emitter_safe_ir` admits no 16-byte atomic), so ra is saved
    // and every a/t register is free once the operands are read.  Operands
    // are read into t-registers first; the call arguments follow the psABI
    // (a 2xXLEN scalar takes the next two argument registers, low half
    // first).

    /// libatomic's memory-model argument (`__ATOMIC_*`).
    fn libatomic_model(ordering: AtomicOrdering) -> i64 {
        match ordering {
            AtomicOrdering::Relaxed => 0,
            AtomicOrdering::Acquire => 2,
            AtomicOrdering::Release => 3,
            AtomicOrdering::AcqRel => 4,
            AtomicOrdering::SeqCst => 5,
        }
    }

    /// Call `name` and invalidate the cached register contents.
    fn emit_libatomic_call(&mut self, name: &str) {
        self.state.emit_fmt(format_args!("    call {name}"));
        self.state.reg_cache.invalidate_all();
    }

    fn emit_atomic_load_128(&mut self, dest: &Value, ptr: &Operand, ordering: AtomicOrdering) {
        // A load model may not release.
        let model = match ordering {
            AtomicOrdering::Release | AtomicOrdering::AcqRel => 5,
            other => Self::libatomic_model(other),
        };
        self.operand_to_t0(ptr);
        self.state.emit("    mv a0, t0");
        self.state.emit_fmt(format_args!("    li a1, {model}"));
        self.emit_libatomic_call("__atomic_load_16");
        self.state.emit("    mv t0, a0");
        self.state.emit("    mv t1, a1");
        self.store_t0_t1_to(dest);
    }

    fn emit_atomic_store_128(&mut self, ptr: &Operand, val: &Operand, ordering: AtomicOrdering) {
        // A store model may not acquire.
        let model = match ordering {
            AtomicOrdering::Acquire | AtomicOrdering::AcqRel => 5,
            other => Self::libatomic_model(other),
        };
        self.operand_to_t0_t1(val);
        self.state.emit("    mv t3, t0");
        self.state.emit("    mv t4, t1");
        self.operand_to_t0(ptr);
        self.state.emit("    mv a0, t0");
        self.state.emit("    mv a1, t3");
        self.state.emit("    mv a2, t4");
        self.state.emit_fmt(format_args!("    li a3, {model}"));
        self.emit_libatomic_call("__atomic_store_16");
    }

    fn emit_atomic_rmw_128(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ordering: AtomicOrdering,
    ) {
        let name = match op {
            AtomicRmwOp::Add => "__atomic_fetch_add_16",
            AtomicRmwOp::Sub => "__atomic_fetch_sub_16",
            AtomicRmwOp::And => "__atomic_fetch_and_16",
            AtomicRmwOp::Or => "__atomic_fetch_or_16",
            AtomicRmwOp::Xor => "__atomic_fetch_xor_16",
            AtomicRmwOp::Nand => "__atomic_fetch_nand_16",
            AtomicRmwOp::Xchg | AtomicRmwOp::TestAndSet => "__atomic_exchange_16",
        };
        if matches!(op, AtomicRmwOp::TestAndSet) {
            self.state.emit("    li t3, 1");
            self.state.emit("    li t4, 0");
        } else {
            self.operand_to_t0_t1(val);
            self.state.emit("    mv t3, t0");
            self.state.emit("    mv t4, t1");
        }
        self.operand_to_t0(ptr);
        self.state.emit("    mv a0, t0");
        self.state.emit("    mv a1, t3");
        self.state.emit("    mv a2, t4");
        self.state.emit_fmt(format_args!(
            "    li a3, {}",
            Self::libatomic_model(ordering)
        ));
        self.emit_libatomic_call(name);
        self.state.emit("    mv t0, a0");
        self.state.emit("    mv t1, a1");
        self.store_t0_t1_to(dest);
    }

    /// `bool __atomic_compare_exchange_16(void *p, void *expected, U128
    /// desired, int success, int failure)`: the expected pair goes through
    /// a 16-byte area below sp, which then holds the old value either way
    /// (libatomic writes it back on failure; on success it equals the
    /// expected one).  No frame slot is addressed while sp is lowered:
    /// every operand is read first.
    #[allow(clippy::too_many_arguments)]
    fn emit_atomic_cmpxchg_128(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        // The failure model may neither release nor exceed the success one.
        let failure = match failure_ordering {
            AtomicOrdering::Release => AtomicOrdering::Relaxed,
            AtomicOrdering::AcqRel => AtomicOrdering::Acquire,
            other => other,
        };
        self.operand_to_t0_t1(expected);
        self.state.emit("    mv t3, t0");
        self.state.emit("    mv t4, t1");
        self.operand_to_t0_t1(desired);
        self.state.emit("    mv a2, t0");
        self.state.emit("    mv a3, t1");
        self.operand_to_t0(ptr);
        self.state.emit("    mv a0, t0");
        self.state.emit("    addi sp, sp, -16");
        self.state.emit("    sd t3, 0(sp)");
        self.state.emit("    sd t4, 8(sp)");
        self.state.emit("    mv a1, sp");
        self.state.emit_fmt(format_args!(
            "    li a4, {}",
            Self::libatomic_model(ordering)
        ));
        self.state.emit_fmt(format_args!(
            "    li a5, {}",
            Self::libatomic_model(failure)
        ));
        self.emit_libatomic_call("__atomic_compare_exchange_16");
        if returns_bool {
            self.state.emit("    mv t0, a0");
            self.state.emit("    addi sp, sp, 16");
            self.store_t0_to(dest);
        } else {
            self.state.emit("    ld t0, 0(sp)");
            self.state.emit("    ld t1, 8(sp)");
            self.state.emit("    addi sp, sp, 16");
            self.store_t0_t1_to(dest);
        }
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        match ordering {
            AtomicOrdering::Relaxed => {}
            AtomicOrdering::Acquire => self.state.emit("    fence r, rw"),
            AtomicOrdering::Release => self.state.emit("    fence rw, w"),
            AtomicOrdering::AcqRel | AtomicOrdering::SeqCst => self.state.emit("    fence rw, rw"),
        }
    }
}
