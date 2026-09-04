//! Machine instruction IR with virtual registers for x86-64.
//!
//! This is a thin intermediate representation between the SSA IR and text
//! assembly. IR instructions are lowered to MachInst sequences that use
//! virtual registers (MachReg::Vreg). A register allocator maps virtual
//! registers to physical registers, inserting spill/reload as needed.
//! The allocated MachInst sequence is then emitted as AT&T assembly text.
//!
//! The MachInst enum only models patterns the codegen actually emits —
//! it is NOT a general x86 instruction set. The `Raw(String)` variant
//! provides an escape hatch for instruction types not yet migrated.

use crate::backend::regalloc::PhysReg;

/// A register operand: either a physical register (pre-colored or allocated)
/// or a virtual register waiting for allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MachReg {
    /// Pre-colored physical register (e.g., rax for division, rcx for shifts).
    /// The register allocator must not assign any vreg to this physreg at
    /// program points where it's pre-colored.
    Phys(PhysReg),
    /// Virtual register identified by IR Value ID. Will be replaced with
    /// Phys after allocation, or spilled to a stack slot.
    Vreg(u32),
}

/// Operand size for instruction encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpSize {
    S8,  // byte (al, bl, r12b, ...)
    S16, // word (ax, bx, r12w, ...)
    S32, // dword (eax, ebx, r12d, ...)
    S64, // qword (rax, rbx, r12, ...)
}

/// ALU operation (two-address form: dst = dst OP src).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Imul, // 2-operand signed multiply (dst *= src)
}

/// Scalar SSE arithmetic operation (VEX three-operand form).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FAluOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// One register-argument move of a [`MachInst::CallTyped`]: place `src`
/// (a resolved GPR home, spill-slot read, or immediate) into the SysV
/// integer argument register `dst_reg`. Self-moves (source already in the
/// target register) are elided by the lowering, never constructed.
#[derive(Debug, Clone)]
pub struct CallArgMove {
    pub src: MachOperand,
    pub dst_reg: PhysReg,
    pub size: OpSize,
}

/// The return-value move of a [`MachInst::CallTyped`]: copy rax to the
/// return value's home (register or spill slot) after the call. Emitted
/// at the return type's width; a width narrower than 8 bytes renders the
/// 32-bit form (`movl %eax, ...`), matching the mature path and the ABI.
#[derive(Debug, Clone)]
pub struct CallRetMove {
    pub dst: MachOperand,
    pub size: OpSize,
}

/// The call form of a [`MachInst::CallTyped`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallTarget {
    /// `call sym` — the symbol string with PLT/version decoration already
    /// applied by the lowering.
    Direct(String),
    /// `call *%reg` — the callee pointer was staged into `reg` by one of
    /// the instruction's own argument moves (or already lived there).
    /// The register is never an ABI argument register (r10/r11 only), so
    /// no argument move can clobber it after its staging.
    Indirect(PhysReg),
}

/// Shift operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftOp {
    Shl,
    Shr, // logical shift right
    Sar, // arithmetic shift right
}

/// Condition code for Jcc, SetCC, CMov.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondCode {
    E,  // equal (ZF=1)
    Ne, // not equal (ZF=0)
    L,  // less (signed) (SF≠OF)
    Le, // less or equal (signed) (ZF=1 or SF≠OF)
    G,  // greater (signed) (ZF=0 and SF=OF)
    Ge, // greater or equal (signed) (SF=OF)
    B,  // below (unsigned) (CF=1)
    Be, // below or equal (unsigned) (CF=1 or ZF=1)
    A,  // above (unsigned) (CF=0 and ZF=0)
    Ae, // above or equal (unsigned) (CF=0)
}

/// An operand in a machine instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachOperand {
    /// Register operand.
    Reg(MachReg),
    /// Immediate constant.
    Imm(i64),
    /// Memory: offset(%base). Base is a register.
    Mem { base: MachReg, offset: i64 },
    /// Indexed memory: offset(%base, %index, scale). Scale is 1/2/4/8.
    MemIndex {
        base: MachReg,
        index: MachReg,
        scale: u8,
        offset: i64,
    },
    /// Stack slot (assigned after register allocation for spills).
    /// The i64 is the offset from the frame pointer (%rbp or %rsp-relative).
    StackSlot(i64),
    /// Address of a stack slot (leaq). The u32 is the IR value ID of the alloca.
    /// During resolve, this becomes `leaq slot(%rbp), %dst` — producing the
    /// address of the alloca, not its contents.
    AllocaAddr(u32),
    /// RIP-relative symbol reference: symbol(%rip).
    RipRel(String),
}

/// A machine instruction with virtual registers.
///
/// These model the x86-64 instruction forms that the codegen actually emits.
/// Not all x86 instructions are represented — only the patterns needed for
/// lowering the LCCC IR. The `Raw` variant provides an escape hatch.
#[derive(Debug, Clone)]
pub enum MachInst {
    // ── Data movement ────────────────────────────────────────────────
    /// mov src, dst (covers reg-reg, imm-reg, mem-reg, reg-mem, etc.)
    Mov {
        src: MachOperand,
        dst: MachOperand,
        size: OpSize,
    },

    /// Scalar SSE move: `movss` (S32) / `movsd` (S64).
    ///
    /// Float values live in the XMM domain (`MachReg::Phys` with an id in
    /// 20..=33, i.e. xmm2..xmm15); GPR<->XMM moves are NOT this instruction
    /// (those are movd/movq and stay on the text path). At most one operand
    /// may be a memory/slot form — x86 has no mem-to-mem SSE move, so the
    /// lowering must not produce one (the slot-homed side needs an xmm
    /// scratch relay, which the mature text path owns).
    ///
    /// Size semantics: S32 selects the single-precision mnemonic (movss),
    /// S64 the double (movsd). Decimal carriers D32/D64 move through the
    /// same bit-exact SSE instructions (mirrors emit_store_impl).
    FMov {
        src: MachOperand,
        dst: MachOperand,
        size: OpSize,
    },

    /// Scalar SSE arithmetic in the VEX three-operand form:
    /// `v{add,sub,mul,div}s{s,d} src2, %src1, %dst` computing
    /// dst = src1 OP src2.
    ///
    /// The three-operand form is the whole point: it has no destructive
    /// destination, so dest-aliases-an-operand needs no staging copy (the
    /// legacy two-address form forced `vmovsd` staging and, for sub/div
    /// whose dest held the RHS, an xmm0 relay). `src1` must be an xmm
    /// register; `src2` may be an xmm register or ONE memory/slot operand
    /// (x86 allows at most one). Size S32 selects the `ss` suffix, S64 `sd`.
    ///
    /// Operand order is semantically load-bearing for Sub/Div (dst =
    /// src1 - src2, src1 / src2): the lowering must keep lhs in src1.
    FAlu {
        op: FAluOp,
        src2: MachOperand,
        src1: MachReg,
        dst: MachReg,
        size: OpSize,
    },

    /// 16-byte integer move: `movdqu src, %xmm0` + `movdqu %xmm0, dst` —
    /// the atomic 128-bit mem-to-mem transfer the scalar Mov cannot express
    /// (x86 has no general mem-to-mem move; the GPR relay needs FOUR S64
    /// moves where the SSE2 scratch pair needs TWO).
    ///
    /// Serves the typed i128 Store and Load lowerings (the Store(other) /
    /// Load(other) census classes): an i128 value lives in a contiguous
    /// 16-byte slot, and a store/load copies that region to/from the
    /// destination address. Both operands must be memory forms (StackSlot
    /// or Mem); register and immediate halves take the scalar Mov path —
    /// the const i128 store splits into two S64 Movs, and the
    /// scratch-unsafe fallback is four S64 Movs through the reserved
    /// rax/rdx.
    ///
    /// Ordering is semantic: the 16-byte source is read in full before any
    /// destination byte is written, so overlapping regions stay correct
    /// (the SSE pair is data-dependent — the store reads the scratch the
    /// load wrote; the GPR fallback orders both loads before both stores).
    ///
    /// `movdqu` (not `movdqa`) is deliberate: frame slots sit at 8 mod 16
    /// under the rsp-mode layout and pointer targets may be packed-member
    /// aligned, so unaligned-safe is the only sound spelling; on modern
    /// cores it costs nothing for aligned data. Clobber: the pre-colored
    /// xmm0 scratch (PhysReg 18) — the isel refuses this instruction when
    /// the scratch is live (xmm_scratch_unsafe) and uses the GPR fallback.
    Mov128 {
        src: MachOperand,
        dst: MachOperand,
    },

    /// movzx src, dst (zero-extend: movzbl, movzwl, movzbq, movzwq)
    Movzx {
        src: MachOperand,
        dst: MachReg,
        from_size: OpSize,
        to_size: OpSize,
    },

    /// movsx src, dst (sign-extend: movsbq, movswq, movslq)
    Movsx {
        src: MachOperand,
        dst: MachReg,
        from_size: OpSize,
        to_size: OpSize,
    },

    // ── Arithmetic ───────────────────────────────────────────────────
    /// Two-address ALU: dst = dst OP src.
    /// x86 form: `{add,sub,and,or,xor,imul} src, dst`
    Alu {
        op: AluOp,
        src: MachOperand,
        dst: MachReg,
        size: OpSize,
    },

    /// Three-operand multiply: dst = src * imm.
    /// x86 form: `imul $imm, src, dst`
    Imul3 {
        imm: i64,
        src: MachReg,
        dst: MachReg,
        size: OpSize,
    },

    /// Unary ALU: dst = OP(dst). Covers neg, not.
    Neg {
        dst: MachReg,
        size: OpSize,
    },
    Not {
        dst: MachReg,
        size: OpSize,
    },

    /// Shift: dst = dst SHIFT amount.
    /// Amount must be Imm or Reg(Phys(RCX)) — x86 constraint.
    Shift {
        op: ShiftOp,
        amount: MachOperand,
        dst: MachReg,
        size: OpSize,
    },

    /// BMI2 three-operand shift: dst = src SHIFT count.  Count may live in
    /// any GPR, the form is non-destructive and leaves EFLAGS untouched.
    /// One µop on every core with BMI2 (uops.info SHLX_R64_R64_R64) against
    /// 2–3 for `Shift` with a CL count on Intel.  Selected by isel only
    /// when `isel::shlx_mode()` allows it (tune row + `-mbmi2`).
    ShiftX {
        op: ShiftOp,
        count: MachReg,
        src: MachReg,
        dst: MachReg,
        size: OpSize,
    },

    /// LEA: dst = base + index*scale + offset (3-address add).
    /// x86 form: `leaq offset(%base, %index, scale), %dst`
    Lea {
        base: MachReg,
        index: Option<(MachReg, u8)>,
        offset: i64,
        dst: MachReg,
    },

    /// `leaq sym(%rip), dst` — the address of a global.
    ///
    /// This needs its own variant because `Mov` with a [`MachOperand::RipRel`]
    /// source means "load FROM the symbol" (`movq sym(%rip), %dst`), whereas a
    /// `GlobalAddr` wants the address ITSELF. Without the distinction the
    /// lowering had no way to express `leaq` and bailed to the text emitter --
    /// 293 of 2162 instructions across the corpus, 13.6%, the single largest
    /// hole in MachInst coverage.
    LeaSym {
        sym: String,
        dst: MachReg,
    },

    // ── Division (implicit rax:rdx) ──────────────────────────────────
    /// Sign-extend rax → rdx:rax. x86: `cqto` (64-bit) or `cltd` (32-bit).
    Cqto {
        size: OpSize,
    },

    /// Zero rdx for unsigned division. x86: `xorl %edx, %edx`.
    XorRdx,

    /// Integer division: rdx:rax / divisor.
    /// Quotient → rax, remainder → rdx.
    /// x86: `idivq divisor` (signed) or `divq divisor` (unsigned).
    Div {
        divisor: MachOperand,
        signed: bool,
        size: OpSize,
    },

    // ── Comparison & flags ───────────────────────────────────────────
    /// Compare: sets flags based on (lhs - rhs).
    /// x86 form: `cmp rhs, lhs` (note: AT&T operand order is reversed).
    Cmp {
        lhs: MachOperand,
        rhs: MachOperand,
        size: OpSize,
    },

    /// Test: sets flags based on (lhs & rhs).
    /// x86 form: `test rhs, lhs`.
    Test {
        lhs: MachOperand,
        rhs: MachOperand,
        size: OpSize,
    },

    /// Set byte from condition code: `setCC %dst_8bit`.
    /// Always writes to the 8-bit sub-register.
    SetCC {
        cc: CondCode,
        dst: MachReg,
    },

    /// Conditional move: dst = cc ? src : dst.
    /// x86 form: `cmovCC src, dst`.
    Cmov {
        cc: CondCode,
        src: MachOperand,
        dst: MachReg,
        size: OpSize,
    },

    // ── Control flow ─────────────────────────────────────────────────
    /// Conditional jump: `jCC target`.
    Jcc {
        cc: CondCode,
        target: String,
    },

    /// Unconditional jump: `jmp target`.
    Jmp {
        target: String,
    },

    /// Block label (pseudo-instruction).
    Label(String),

    /// Direct function call: `call target`. Clobbers caller-saved regs.
    Call {
        target: String,
    },

    /// Fully typed call — the SysV integer-register-call contract as ONE
    /// machine instruction, the way an MIR `CALL` or a Cranelift `call`
    /// instruction models it: declared clobbers, declared uses, atomic
    /// under the replay fallback. Direct and indirect targets share the
    /// contract; only the final call form differs ([`CallTarget`]).
    ///
    /// Emission order (all stages are part of this instruction):
    ///   1. `caller_saves`: spill every caller-saved register that holds a
    ///      value live across the call to its spill slot. Computed by the
    ///      lowering from the same `caller_save_intervals` data the mature
    ///      path's Phase 2b consumes, at the call's program point.
    ///   2. `args`: one move per register argument, placing its (already
    ///      resolved) source into the SysV integer argument register.
    ///      Sources are GPR homes, spill-slot reads, or 64-bit immediates;
    ///      a zero immediate emits as `xorl reg, reg` (3 bytes vs 7, the
    ///      GCC/Clang/ICC form) and an immediate outside the sign-extended
    ///      imm32 window emits as `movabsq imm, reg` (the only sound way to
    ///      name a full 64-bit constant — inline truncation is not an
    ///      option and GAS rejects the raw operand). For an indirect call,
    ///      the callee-pointer staging move (`slot → %r10`) is part of this
    ///      same ordered list, so the topological reader-before-writer rule
    ///      protects its source against every argument move for free.
    ///      The moves are execution-ordered by the lowering so no source is
    ///      clobbered by an earlier move's destination; register-exchange
    ///      cycles never reach this variant (they are rejected to the text
    ///      path, which owns the hazard-spill area machinery).
    ///   3. the call itself — `call target` for a direct target (PLT and
    ///      version decoration baked into the symbol), `call *%reg` for an
    ///      indirect one. Under an external retpoline thunk the register is
    ///      pinned to r10 (the thunk symbol encodes the register:
    ///      `__x86_indirect_thunk_r10`); the emit-side gate refuses any
    ///      other register in that mode rather than emitting an unpatchable
    ///      call shape.
    ///   4. `ret`: copy rax to the return value's home (register or slot).
    ///      None for void calls.
    ///   5. `caller_saves` again, reversed direction: restore each spilled
    ///      register from its slot (the mature path's Phase 2b tail).
    ///
    /// Declared clobbers: the six SysV integer argument registers, rax,
    /// rdx, r8-r11 and every xmm — the full caller-clobber set. Declared
    /// uses: the argument sources, the callee-pointer staging source (all
    /// read before any clobber) and the spill slots (memory, not
    /// registers). Because the lowering flushes the buffer immediately
    /// around the call run, no allocated vreg can coexist with this
    /// instruction — the pre-colored argument/return registers are
    /// therefore conflict-free by construction.
    ///
    /// The subset that may reach this variant (enforced twice — emit-side
    /// gate and builder): non-variadic, no sret/struct/i128/F128/x87
    /// arguments, at most six integer/pointer arguments of width 4 or 8,
    /// integer or pointer return; the target is a named symbol (direct) or
    /// a GPR/slot-homed pointer value (indirect, staged into r10/r11).
    /// Everything else stays on the mature path, which owns the general
    /// contract, the paravirt patchable `call *sym(%rip)` form and the
    /// hazard-spill machinery.
    CallTyped {
        /// (register, spill slot offset) pairs saved before / restored after.
        caller_saves: Vec<(PhysReg, i64)>,
        /// Register-argument moves in execution order (indirect callee
        /// staging included, see above).
        args: Vec<CallArgMove>,
        /// Call target: direct symbol or indirect register.
        target: CallTarget,
        /// Return-value home: rax is copied here after the call.
        ret: Option<CallRetMove>,
    },

    /// Return: `ret`.
    Ret,

    // ── Passthrough ──────────────────────────────────────────────────
    /// Raw assembly text for instructions not yet migrated to MachInst.
    /// The register allocator treats this as a barrier (invalidates all vregs).
    Raw(String),
}

// ── Well-known physical register IDs ─────────────────────────────────────
//
// PhysReg IDs match the existing codegen convention (emit.rs:79-88).
// IDs 0 and 7 are NEW — rax/rcx are scratch in the existing accumulator
// model but need pre-coloring in the MachInst model for division/shifts.
// The MachInst emitter (machinst_emit.rs) handles all IDs including 0/7.

/// rax — accumulator, division result, function return value.
/// NOT in the existing allocatable pool (used as scratch).
pub const RAX: PhysReg = PhysReg(0);
/// rcx — shift count (%cl), 4th SysV argument register.
/// NOT in the existing allocatable pool (used as scratch).
pub const RCX: PhysReg = PhysReg(7);
/// rdx — division high half, 3rd SysV argument register.
pub const RDX: PhysReg = PhysReg(16);

// Callee-saved registers (available for allocation, preserved across calls):
pub const RBX: PhysReg = PhysReg(1);
pub const R12: PhysReg = PhysReg(2);
pub const R13: PhysReg = PhysReg(3);
pub const R14: PhysReg = PhysReg(4);
pub const R15: PhysReg = PhysReg(5);
pub const RBP: PhysReg = PhysReg(6);

// Caller-saved registers (destroyed by calls, available between calls):
pub const R11: PhysReg = PhysReg(10);
pub const R10: PhysReg = PhysReg(11);
pub const R8: PhysReg = PhysReg(12);
pub const R9: PhysReg = PhysReg(13);
pub const RDI: PhysReg = PhysReg(14);
pub const RSI: PhysReg = PhysReg(15);

/// All GPR registers that the MachInst allocator can assign to virtual registers.
/// Excludes rax (0) and rcx (7) which are reserved as scratch.
pub const MACHINST_ALLOCATABLE_GPRS: &[PhysReg] = &[
    // Callee-saved (survive calls):
    RBX, R12, R13, R14, R15, RBP,
    // Caller-saved (destroyed by calls, only for non-call-spanning values):
    R11, R10, R8, R9, RDI, RSI, RDX,
];

/// The six SysV AMD64 integer argument registers, in ABI order:
/// arg0=rdi, arg1=rsi, arg2=rdx, arg3=rcx, arg4=r8, arg5=r9.
/// IDs follow the codegen convention (calls.rs: rdi=14, rsi=15, rdx=16,
/// rcx=7, r8=12, r9=13).
pub const SYSCALL_ARG_REGS: [PhysReg; 6] = [
    PhysReg(14), // rdi
    PhysReg(15), // rsi
    PhysReg(16), // rdx
    PhysReg(7),  // rcx
    PhysReg(12), // r8
    PhysReg(13), // r9
];

/// xmm0 — pre-colored float scratch. The allocator's XMM pool starts at
/// xmm2 (PhysReg 20), so xmm0/xmm1 are conflict-free by construction: no
/// allocated float value can ever live there. Used by the lowering to
/// relay between two memory operands (x86 has no mem-to-mem SSE move) —
/// slot-homed float loads/stores and float compare staging.
pub const XMM0_SCRATCH: PhysReg = PhysReg(18);
/// xmm1 — second pre-colored float scratch (two-operand compares).
pub const XMM1_SCRATCH: PhysReg = PhysReg(19);

impl MachReg {
    /// Returns true if this is a virtual register (needs allocation).
    pub fn is_vreg(&self) -> bool {
        matches!(self, MachReg::Vreg(_))
    }

    /// Returns true if this is a pre-colored physical register.
    pub fn is_phys(&self) -> bool {
        matches!(self, MachReg::Phys(_))
    }

    /// Get the virtual register ID, if this is a vreg.
    pub fn vreg_id(&self) -> Option<u32> {
        match self {
            MachReg::Vreg(id) => Some(*id),
            _ => None,
        }
    }

    /// Get the physical register, if this is pre-colored.
    pub fn phys(&self) -> Option<PhysReg> {
        match self {
            MachReg::Phys(r) => Some(*r),
            _ => None,
        }
    }
}

impl OpSize {
    /// Convert an IrType to an OpSize.
    pub fn from_ir_type(ty: crate::common::types::IrType) -> Self {
        use crate::common::types::IrType;
        match ty {
            IrType::I8 | IrType::U8 => OpSize::S8,
            IrType::I16 | IrType::U16 => OpSize::S16,
            IrType::I32 | IrType::U32 | IrType::F32 => OpSize::S32,
            _ => OpSize::S64, // I64, U64, F64, Ptr, etc.
        }
    }

    /// AT&T instruction suffix for this size.
    pub fn suffix(&self) -> &'static str {
        match self {
            OpSize::S8 => "b",
            OpSize::S16 => "w",
            OpSize::S32 => "l",
            OpSize::S64 => "q",
        }
    }
}
