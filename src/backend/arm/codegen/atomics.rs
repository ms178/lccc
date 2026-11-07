//! ArmCodegen: atomic operations.
//!
//! Register contract (shared with `ArmCodegen::arm_pool_scratch` and the
//! allocator's atomic-pointer eligibility):
//!
//! * The 8- to 64-bit sequences write x0 (the loaded old value), x2 (an
//!   expected value masked to the object's width), x3 (the computed new
//!   value) and w4 (the store-exclusive status), and stage an operand that
//!   has no usable home in x1 (pointer), x2 (value, expected) or x3
//!   (desired) through x0.  None of x0-x3 is ever an allocation home, and x4
//!   leaves the caller-saved pool of every function with an RMW or a
//!   compare-exchange, so an operand homed anywhere else is read in place:
//!   `ldaxr w0, [x5]` instead of a stack reload of the pointer.  A home that
//!   is one of the written registers anyway (defensive: no current pool
//!   offers one) is staged like a slot.
//! * 128-bit operations are LDXP/STXP loops -- Clang 23.1.5's expansion
//!   without LSE; libatomic's lock-free AArch64 16-byte routines use the
//!   same exclusive pair, so both interoperate on one object.  The
//!   allocator treats a 16-byte atomic as a call point
//!   (`liveness::instruction_is_call_point`), so no value lives across it
//!   in a caller-saved register: once the operands are read the sequence
//!   owns x0-x3, x9-x12 and x16-x17 (x9 pointer, x2:x3 value or desired
//!   pair, x10:x11 expected pair, x0:x1 loaded pair, x16:x17 new pair, w12
//!   status; no store-exclusive status register equals one of its data or
//!   address registers, which the architecture leaves CONSTRAINED
//!   UNPREDICTABLE).  A 16-byte load also stores the pair it read back:
//!   only a successful STXP makes the LDXP pair single-copy atomic.

use super::emit::{ArmCodegen, callee_saved_name, is_arm_fp_phys};
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, IrConst, Operand, Value};

/// Registers every 8- to 64-bit atomic sequence may write (see the module
/// documentation); an operand homed in one of them is staged instead.
const NARROW_CLOBBERS: [u8; 5] = [0, 1, 2, 3, 4];

/// Register number standing for the zero register in an operand position.
const ZR: u8 = 31;

fn is_128(ty: IrType) -> bool {
    matches!(ty, IrType::I128 | IrType::U128)
}

fn acquires(ordering: AtomicOrdering) -> bool {
    matches!(
        ordering,
        AtomicOrdering::Acquire | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
    )
}

fn releases(ordering: AtomicOrdering) -> bool {
    matches!(
        ordering,
        AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst
    )
}

/// `w`/`x` view of general register `n` (31 is the zero register).
fn gpr(n: u8, wide: bool) -> String {
    match (n, wide) {
        (ZR, true) => "xzr".to_string(),
        (ZR, false) => "wzr".to_string(),
        (n, true) => format!("x{n}"),
        (n, false) => format!("w{n}"),
    }
}

/// Whether the 8- to 64-bit sequence for `ty` names X registers: exactly the
/// 8-byte types.  The register width IS the access width of LDAXR/STLXR,
/// LDAR/STLR and LDR/STR, so a 4-byte `float` must take W registers (it
/// used to fall into the X default: an 8-byte exclusive pair on a 4-byte
/// object, writing back the 4 bytes after it).
fn wide_regs(ty: IrType) -> bool {
    matches!(ty, IrType::I64 | IrType::U64 | IrType::Ptr | IrType::F64)
}

impl ArmCodegen {
    /// Canonicalize an exclusive-operation result in X0 to the IR type.
    fn canonicalize_atomic_x0_result(&mut self, ty: IrType) {
        match ty {
            IrType::I8 => self.state.emit("    sxtb x0, w0"),
            IrType::I16 => self.state.emit("    sxth x0, w0"),
            IrType::I32 => self.state.emit("    sxtw x0, w0"),
            // Byte/halfword loads and every W-register write already clear
            // the upper bits for unsigned types.
            _ => {}
        }
    }

    /// The general register `op` lives in, when it has a GPR home outside
    /// `clobbered`.
    fn atomic_home(&self, op: &Operand, clobbered: &[u8]) -> Option<u8> {
        let Operand::Value(v) = op else {
            return None;
        };
        if self.state.is_alloca(v.0) {
            return None;
        }
        let reg = *self.reg_assignments.get(&v.0)?;
        (!is_arm_fp_phys(reg) && reg.0 < ZR && !clobbered.contains(&reg.0)).then_some(reg.0)
    }

    /// The register an atomic sequence reads `op` from: its own home (see
    /// [`Self::atomic_home`]), the zero register for a zero constant when
    /// `allow_zr`, else `scratch` loaded through x0.  Operands are fetched
    /// one at a time, so an operand staged earlier must sit in a register
    /// other than x0 (`scratch != 0`) unless it is the last one fetched.
    fn atomic_operand_reg(&mut self, op: &Operand, scratch: u8, allow_zr: bool) -> u8 {
        if allow_zr && Self::is_all_zero_bits(op) {
            return ZR;
        }
        if let Some(home) = self.atomic_home(op, &NARROW_CLOBBERS) {
            return home;
        }
        if let Some(value) = Self::atomic_int_const(op) {
            // Straight into the scratch: through x0 the constant left a
            // dead `mov x0, #c` the loop's load-exclusive overwrites.
            if scratch == 0 {
                self.state.reg_cache.invalidate_acc();
            }
            self.emit_load_imm64(&format!("x{scratch}"), value);
            return scratch;
        }
        self.operand_to_x0(op);
        if scratch != 0 {
            self.state.emit_fmt(format_args!("    mov x{scratch}, x0"));
        }
        scratch
    }

    /// A constant whose bits are all zero (never `-0.0`), which the zero
    /// register supplies in a data operand position.
    fn is_all_zero_bits(op: &Operand) -> bool {
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

    pub(super) fn emit_atomic_rmw_impl(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        if is_128(ty) {
            self.emit_atomic_rmw_128(dest, op, ptr, val, ordering);
            return;
        }
        let wide = wide_regs(ty);
        // The new value: a register, or an immediate the ALU op encodes.
        let rhs = match op {
            AtomicRmwOp::TestAndSet => None,
            AtomicRmwOp::Add | AtomicRmwOp::Sub => Self::const_as_imm12(val)
                .map(|imm| format!("#{imm}"))
                .or_else(|| Self::const_as_negated_imm12(val).map(|imm| format!("#{imm}"))),
            AtomicRmwOp::And | AtomicRmwOp::Or | AtomicRmwOp::Xor => {
                Self::const_as_logical_imm(val, if wide { 64 } else { 32 })
                    .map(|imm| format!("#{imm:#x}"))
            }
            AtomicRmwOp::Nand | AtomicRmwOp::Xchg => None,
        };
        // `x - imm` for a negative addend and `x + imm` for a negative
        // subtrahend.
        let op_word = match (op, &rhs) {
            (AtomicRmwOp::Add, Some(_)) if Self::const_as_imm12(val).is_none() => "sub",
            (AtomicRmwOp::Sub, Some(_)) if Self::const_as_imm12(val).is_none() => "add",
            (AtomicRmwOp::Add, _) => "add",
            (AtomicRmwOp::Sub, _) => "sub",
            (AtomicRmwOp::And, _) | (AtomicRmwOp::Nand, _) => "and",
            (AtomicRmwOp::Or, _) => "orr",
            (AtomicRmwOp::Xor, _) => "eor",
            (AtomicRmwOp::Xchg, _) | (AtomicRmwOp::TestAndSet, _) => "",
        };
        // A zero operand is the zero register in every ALU form used here
        // (ADD/SUB/AND/ORR/EOR shifted-register and STXR data operands).
        let val_reg = match (op, &rhs) {
            (AtomicRmwOp::TestAndSet, _) | (_, Some(_)) => None,
            (_, None) => Some(self.atomic_operand_reg(val, 2, true)),
        };
        let ptr_reg = self.atomic_operand_reg(ptr, 1, false);
        if matches!(op, AtomicRmwOp::TestAndSet) {
            self.state.emit("    mov w3, #1");
        }

        let (ldxr, stxr, _) = Self::exclusive_instrs(ty, ordering);
        let old = gpr(0, wide);
        let tmp = gpr(3, wide);
        let loop_label = self.state.fresh_label("atomic");
        self.state.emit_fmt(format_args!("{loop_label}:"));
        self.state
            .emit_fmt(format_args!("    {ldxr} {old}, [x{ptr_reg}]"));
        let stored = match op {
            AtomicRmwOp::TestAndSet => "w3".to_string(),
            AtomicRmwOp::Xchg => gpr(val_reg.unwrap_or(ZR), wide),
            _ => {
                let operand = match (&rhs, val_reg) {
                    (Some(imm), _) => imm.clone(),
                    (None, Some(reg)) => gpr(reg, wide),
                    (None, None) => unreachable!("a register-operand RMW stages its value"),
                };
                self.state
                    .emit_fmt(format_args!("    {op_word} {tmp}, {old}, {operand}"));
                if matches!(op, AtomicRmwOp::Nand) {
                    self.state.emit_fmt(format_args!("    mvn {tmp}, {tmp}"));
                }
                tmp.clone()
            }
        };
        self.state
            .emit_fmt(format_args!("    {stxr} w4, {stored}, [x{ptr_reg}]"));
        self.state
            .emit_fmt(format_args!("    cbnz w4, {loop_label}"));
        if !matches!(op, AtomicRmwOp::TestAndSet) {
            self.canonicalize_atomic_x0_result(ty);
        }
        self.store_x0_to(dest);
    }

    /// An integer constant operand's value, sign-extended as
    /// `operand_to_x0` materializes it.
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

    /// The magnitude of a negative constant whose negation is an imm12
    /// (`fetch_add(p, -1)` subtracts 1).
    fn const_as_negated_imm12(op: &Operand) -> Option<i64> {
        let Operand::Const(c) = op else {
            return None;
        };
        let value = match c {
            IrConst::I8(v) => i64::from(*v),
            IrConst::I16(v) => i64::from(*v),
            IrConst::I32(v) => i64::from(*v),
            IrConst::I64(v) => *v,
            _ => return None,
        };
        value
            .checked_neg()
            .filter(|magnitude| (1..=4095).contains(magnitude))
    }

    pub(super) fn emit_atomic_cmpxchg_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        success_ordering: AtomicOrdering,
        _failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        if is_128(ty) {
            self.emit_atomic_cmpxchg_128(
                dest,
                ptr,
                expected,
                desired,
                success_ordering,
                returns_bool,
            );
            return;
        }
        let wide = wide_regs(ty);
        let desired_reg = self.atomic_operand_reg(desired, 3, true);
        // LDXRB/LDXRH zero-extend, so the expected value is compared at the
        // atomic object's width: a narrow one is masked into x2 (its home
        // keeps the canonical value).  W and X compares read exactly the
        // object's bits.
        let expected_reg = match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => {
                let source = self.atomic_operand_reg(expected, 2, true);
                if source == ZR {
                    ZR
                } else {
                    let mask = if matches!(ty, IrType::I8 | IrType::U8) {
                        0xff
                    } else {
                        0xffff
                    };
                    self.state.emit_fmt(format_args!(
                        "    and w2, {}, #{mask:#x}",
                        gpr(source, false)
                    ));
                    2
                }
            }
            _ => self.atomic_operand_reg(expected, 2, true),
        };
        let ptr_reg = self.atomic_operand_reg(ptr, 1, false);

        let (ldxr, stxr, _) = Self::exclusive_instrs(ty, success_ordering);
        let old = gpr(0, wide);
        let loop_label = self.state.fresh_label("cas_loop");
        let fail_label = self.state.fresh_label("cas_fail");
        self.state.emit_fmt(format_args!("{loop_label}:"));
        self.state
            .emit_fmt(format_args!("    {ldxr} {old}, [x{ptr_reg}]"));
        self.state
            .emit_fmt(format_args!("    cmp {old}, {}", gpr(expected_reg, wide)));
        self.state.emit_fmt(format_args!("    b.ne {fail_label}"));
        self.state.emit_fmt(format_args!(
            "    {stxr} w4, {}, [x{ptr_reg}]",
            gpr(desired_reg, wide)
        ));
        self.state
            .emit_fmt(format_args!("    cbnz w4, {loop_label}"));
        // Both paths meet here, the flags still holding the last compare
        // (`eq` exactly on success: STXR and CBNZ leave NZCV alone).  The
        // failure path leaves the exclusive monitor armed, as GCC 16.2 and
        // the Linux LL/SC `cmpxchg` do: nothing requires a CLREX after a
        // load-exclusive (the next load-exclusive re-arms the monitor and
        // a context switch clears it), so the success path pays no `b`
        // around one and the failure path no extra instruction.
        self.state.emit_fmt(format_args!("{fail_label}:"));
        if returns_bool {
            self.state.emit("    cset x0, eq");
        } else {
            self.canonicalize_atomic_x0_result(ty);
        }
        self.store_x0_to(dest);
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        if is_128(ty) {
            self.emit_atomic_load_128(dest, ptr, ordering);
            return;
        }
        let ptr_reg = self.atomic_operand_reg(ptr, 0, false);
        let need_acquire = acquires(ordering);
        let instr = match (ty, need_acquire) {
            (IrType::I8 | IrType::U8, true) => "ldarb",
            (IrType::I8 | IrType::U8, false) => "ldrb",
            (IrType::I16 | IrType::U16, true) => "ldarh",
            (IrType::I16 | IrType::U16, false) => "ldrh",
            (_, true) => "ldar",
            (_, false) => "ldr",
        };
        self.state.emit_fmt(format_args!(
            "    {instr} {}, [x{ptr_reg}]",
            gpr(0, wide_regs(ty))
        ));
        self.canonicalize_atomic_x0_result(ty);
        self.store_x0_to(dest);
    }

    pub(super) fn emit_atomic_store_impl(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        if is_128(ty) {
            self.emit_atomic_store_128(ptr, val, ordering);
            return;
        }
        let val_reg = self.atomic_operand_reg(val, 1, true);
        let ptr_reg = self.atomic_operand_reg(ptr, 0, false);
        let need_release = releases(ordering);
        let instr = match (ty, need_release) {
            (IrType::I8 | IrType::U8, true) => "stlrb",
            (IrType::I8 | IrType::U8, false) => "strb",
            (IrType::I16 | IrType::U16, true) => "stlrh",
            (IrType::I16 | IrType::U16, false) => "strh",
            (_, true) => "stlr",
            (_, false) => "str",
        };
        self.state.emit_fmt(format_args!(
            "    {instr} {}, [x{ptr_reg}]",
            gpr(val_reg, wide_regs(ty))
        ));
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        match ordering {
            AtomicOrdering::Relaxed => {}
            AtomicOrdering::Acquire => self.state.emit("    dmb ishld"),
            // A C release fence orders earlier loads as well as stores before
            // a following publishing store; DMB ISHST would omit load->store.
            AtomicOrdering::Release | AtomicOrdering::AcqRel | AtomicOrdering::SeqCst => {
                self.state.emit("    dmb ish")
            }
        }
    }

    // ---- 128-bit: LDXP/STXP loops ----

    /// Stage a 16-byte operand in `lo`:`hi` (through x0:x1).
    fn atomic128_pair(&mut self, op: &Operand, lo: &str, hi: &str) {
        self.operand_to_x0_x1(op);
        self.state.emit_fmt(format_args!("    mov {lo}, x0"));
        self.state.emit_fmt(format_args!("    mov {hi}, x1"));
    }

    /// Stage the pointer in x9; called after every pair operand, whose
    /// loads may use x9 as their own scratch.
    fn atomic128_ptr(&mut self, ptr: &Operand) {
        if let Some(home) = self.atomic_home(ptr, &[]) {
            let name = callee_saved_name(crate::backend::regalloc::PhysReg(home));
            self.state.emit_fmt(format_args!("    mov x9, {name}"));
        } else {
            self.operand_to_x0(ptr);
            self.state.emit("    mov x9, x0");
        }
    }

    fn exclusive_pair_instrs(ordering: AtomicOrdering) -> (&'static str, &'static str) {
        (
            if acquires(ordering) { "ldaxp" } else { "ldxp" },
            if releases(ordering) { "stlxp" } else { "stxp" },
        )
    }

    fn emit_atomic_load_128(&mut self, dest: &Value, ptr: &Operand, ordering: AtomicOrdering) {
        self.atomic128_ptr(ptr);
        // A load stores back the pair it read (only a successful STXP makes
        // the pair single-copy atomic); a sequentially consistent load
        // releases with it, as Clang's does.
        let ldxp = if acquires(ordering) { "ldaxp" } else { "ldxp" };
        let stxp = if matches!(ordering, AtomicOrdering::SeqCst) {
            "stlxp"
        } else {
            "stxp"
        };
        let loop_label = self.state.fresh_label("atomic128_ld");
        self.state.emit_fmt(format_args!("{loop_label}:"));
        self.state.emit_fmt(format_args!("    {ldxp} x0, x1, [x9]"));
        self.state
            .emit_fmt(format_args!("    {stxp} w12, x0, x1, [x9]"));
        self.state
            .emit_fmt(format_args!("    cbnz w12, {loop_label}"));
        self.store_x0_x1_to(dest);
    }

    fn emit_atomic_store_128(&mut self, ptr: &Operand, val: &Operand, ordering: AtomicOrdering) {
        self.atomic128_pair(val, "x2", "x3");
        self.atomic128_ptr(ptr);
        // The exclusive load claims the monitor; its pair is discarded.  A
        // sequentially consistent store also acquires with it (Clang).
        let ldxp = if matches!(ordering, AtomicOrdering::SeqCst) {
            "ldaxp"
        } else {
            "ldxp"
        };
        let stxp = if releases(ordering) { "stlxp" } else { "stxp" };
        let loop_label = self.state.fresh_label("atomic128_st");
        self.state.emit_fmt(format_args!("{loop_label}:"));
        self.state.emit_fmt(format_args!("    {ldxp} x0, x1, [x9]"));
        self.state
            .emit_fmt(format_args!("    {stxp} w12, x2, x3, [x9]"));
        self.state
            .emit_fmt(format_args!("    cbnz w12, {loop_label}"));
    }

    fn emit_atomic_rmw_128(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ordering: AtomicOrdering,
    ) {
        if matches!(op, AtomicRmwOp::TestAndSet) {
            self.state.emit("    mov x2, #1");
            self.state.emit("    mov x3, #0");
        } else {
            self.atomic128_pair(val, "x2", "x3");
        }
        self.atomic128_ptr(ptr);
        let (ldxp, stxp) = Self::exclusive_pair_instrs(ordering);
        let loop_label = self.state.fresh_label("atomic128");
        self.state.emit_fmt(format_args!("{loop_label}:"));
        self.state.emit_fmt(format_args!("    {ldxp} x0, x1, [x9]"));
        let (new_lo, new_hi) = match op {
            AtomicRmwOp::Xchg | AtomicRmwOp::TestAndSet => ("x2", "x3"),
            AtomicRmwOp::Add => {
                self.state.emit("    adds x16, x0, x2");
                self.state.emit("    adc x17, x1, x3");
                ("x16", "x17")
            }
            AtomicRmwOp::Sub => {
                self.state.emit("    subs x16, x0, x2");
                self.state.emit("    sbc x17, x1, x3");
                ("x16", "x17")
            }
            AtomicRmwOp::And | AtomicRmwOp::Or | AtomicRmwOp::Xor | AtomicRmwOp::Nand => {
                let word = match op {
                    AtomicRmwOp::Or => "orr",
                    AtomicRmwOp::Xor => "eor",
                    _ => "and",
                };
                self.state.emit_fmt(format_args!("    {word} x16, x0, x2"));
                self.state.emit_fmt(format_args!("    {word} x17, x1, x3"));
                if matches!(op, AtomicRmwOp::Nand) {
                    self.state.emit("    mvn x16, x16");
                    self.state.emit("    mvn x17, x17");
                }
                ("x16", "x17")
            }
        };
        self.state
            .emit_fmt(format_args!("    {stxp} w12, {new_lo}, {new_hi}, [x9]"));
        self.state
            .emit_fmt(format_args!("    cbnz w12, {loop_label}"));
        self.store_x0_x1_to(dest);
    }

    fn emit_atomic_cmpxchg_128(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        self.atomic128_pair(desired, "x2", "x3");
        self.atomic128_pair(expected, "x10", "x11");
        self.atomic128_ptr(ptr);
        let (ldxp, stxp) = Self::exclusive_pair_instrs(ordering);
        let loop_label = self.state.fresh_label("cas128_loop");
        let fail_label = self.state.fresh_label("cas128_fail");
        let done_label = self.state.fresh_label("cas128_done");
        self.state.emit_fmt(format_args!("{loop_label}:"));
        self.state.emit_fmt(format_args!("    {ldxp} x0, x1, [x9]"));
        self.state.emit("    cmp x0, x10");
        self.state.emit("    ccmp x1, x11, #0, eq");
        self.state.emit_fmt(format_args!("    b.ne {fail_label}"));
        self.state
            .emit_fmt(format_args!("    {stxp} w12, x2, x3, [x9]"));
        self.state
            .emit_fmt(format_args!("    cbnz w12, {loop_label}"));
        if returns_bool {
            self.state.emit("    mov x0, #1");
        }
        self.state.emit_fmt(format_args!("    b {done_label}"));
        // A failed compare stores the pair it read back: the comparison
        // only stands if that pair was read single-copy atomically.
        self.state.emit_fmt(format_args!("{fail_label}:"));
        self.state
            .emit_fmt(format_args!("    {stxp} w12, x0, x1, [x9]"));
        self.state
            .emit_fmt(format_args!("    cbnz w12, {loop_label}"));
        if returns_bool {
            self.state.emit("    mov x0, #0");
        }
        self.state.emit_fmt(format_args!("{done_label}:"));
        if returns_bool {
            self.store_x0_to(dest);
        } else {
            self.store_x0_x1_to(dest);
        }
    }
}
