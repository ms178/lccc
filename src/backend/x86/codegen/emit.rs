use super::machinst::{MachInst, MachOperand, MachReg, OpSize};
use crate::backend::call_abi::{CallAbiConfig, CallArgClass, ParamClass};
use crate::backend::cast::FloatOp;
use crate::backend::common::PtrDirective;
use crate::backend::inline_asm::emit_inline_asm_common;
use crate::backend::regalloc::PhysReg;
use crate::backend::state::{CodegenState, StackSlot};
use crate::backend::traits::{
    ArchCodegen, MAX_JUMP_TABLE_RANGE, MIN_JUMP_TABLE_CASES, MIN_JUMP_TABLE_DENSITY_PERCENT,
};
use crate::common::fp_contract::FpContract;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::delegate_to_impl;
use crate::ir::reexports::{
    AtomicOrdering, AtomicRmwOp, BlockId, IntrinsicOp, IrBinOp, IrCmpOp, IrConst, IrFunction,
    IrUnaryOp, Operand, Value,
};

/// x86-64 callee-saved registers available for register allocation.
/// System V AMD64 ABI callee-saved: rbx, r12, r13, r14, r15.
/// rbp is the frame pointer and cannot be allocated (unless FPO mode).
/// PhysReg encoding: 1=rbx, 2=r12, 3=r13, 4=r14, 5=r15.
pub(super) const X86_CALLEE_SAVED: [PhysReg; 5] =
    [PhysReg(1), PhysReg(2), PhysReg(3), PhysReg(4), PhysReg(5)];

/// Extended callee-saved list including rbp (used when frame pointer is omitted).
/// PhysReg(6) = rbp.
pub(super) const X86_CALLEE_SAVED_WITH_RBP: [PhysReg; 6] = [
    PhysReg(1),
    PhysReg(2),
    PhysReg(3),
    PhysReg(4),
    PhysReg(5),
    PhysReg(6),
];

/// x86-64 caller-saved registers available for allocation to values that
/// do NOT span function calls. These registers are destroyed by calls, so
/// they can only hold values between calls. No prologue/epilogue save is needed.
///
/// PhysReg encoding: 10=r11, 11=r10, 12=r8, 13=r9, 14=rdi, 15=rsi
/// (IDs 10+ to avoid overlap with callee-saved 1-5)
///
/// rdi and rsi are included because the SysV ABI makes them caller-saved.
/// After the prologue stores incoming parameters from rdi/rsi to their stack
/// slots, these registers are free for use within the function body.
/// Memcpy (rep movsb) and VaArg operations that clobber rdi/rsi are treated
/// as call points by the liveness analysis, so values allocated to rdi/rsi
/// will never have live ranges spanning those operations.
pub(super) const X86_CALLER_SAVED: [PhysReg; 6] = [
    PhysReg(10),
    PhysReg(11),
    PhysReg(12),
    PhysReg(13),
    PhysReg(14),
    PhysReg(15),
];

/// x86-64 XMM registers available for F64 allocation.
/// Caller-saved (SysV ABI), only for values that don't span calls.
/// xmm0/xmm1 are reserved for the accumulator pattern and FP return values.
pub(super) const X86_XMM_REGS: [PhysReg; 14] = [
    PhysReg(20),
    PhysReg(21),
    PhysReg(22),
    PhysReg(23),
    PhysReg(24),
    PhysReg(25),
    PhysReg(26),
    PhysReg(27),
    PhysReg(28),
    PhysReg(29),
    PhysReg(30),
    PhysReg(31),
    PhysReg(32),
    PhysReg(33),
];

/// Convert a 64-bit register name string to its 32-bit sub-register name.
/// Used for `xorl %eXX, %eXX` zeroing idiom (shorter encoding, breaks dependencies).
pub(super) fn reg_name_to_32(name: &str) -> &'static str {
    match name {
        "rax" => "eax",
        "rbx" => "ebx",
        "rcx" => "ecx",
        "rdx" => "edx",
        "rsi" => "esi",
        "rdi" => "edi",
        "rsp" => "esp",
        "rbp" => "ebp",
        "r8" => "r8d",
        "r9" => "r9d",
        "r10" => "r10d",
        "r11" => "r11d",
        "r12" => "r12d",
        "r13" => "r13d",
        "r14" => "r14d",
        "r15" => "r15d",
        _ => unreachable!("invalid 64-bit register name: {}", name),
    }
}

/// Map a PhysReg index to its x86-64 register name.
/// Handles both callee-saved (1-5) and caller-saved (10-15) registers.
#[inline]
pub(super) fn phys_reg_name(reg: PhysReg) -> &'static str {
    match reg.0 {
        1 => "rbx",
        2 => "r12",
        3 => "r13",
        4 => "r14",
        5 => "r15",
        6 => "rbp",
        10 => "r11",
        11 => "r10",
        12 => "r8",
        13 => "r9",
        14 => "rdi",
        15 => "rsi",
        16 => "rdx",
        // XMM registers for F64 allocation
        20 => "xmm2",
        21 => "xmm3",
        22 => "xmm4",
        23 => "xmm5",
        24 => "xmm6",
        25 => "xmm7",
        // xmm8-xmm15 are additional caller-saved F64 homes. Codegen
        // scratch only ever touches xmm0/xmm1 (and xmm2 for pblendvb/VNNI),
        // so the upper bank is a safe, stable home; the assembler already
        // encodes them (REX.R/X/B extension bits).
        26 => "xmm8",
        27 => "xmm9",
        28 => "xmm10",
        29 => "xmm11",
        30 => "xmm12",
        31 => "xmm13",
        32 => "xmm14",
        33 => "xmm15",
        _ => unreachable!("invalid x86 register index {}", reg.0),
    }
}

/// Check if a PhysReg is an XMM register (used for F64 values).
#[inline]
pub(super) fn is_xmm_reg(reg: PhysReg) -> bool {
    reg.0 >= 20 && reg.0 <= 33
}

/// Map an XMM allocator register to the corresponding 256-bit AVX name.
/// PhysReg identity is shared across XMM/YMM widths; the vector intrinsic
/// itself supplies the width, so width-specific emission belongs here rather
/// than in the allocator.
#[inline]
pub(super) fn phys_reg_name_256(reg: PhysReg) -> &'static str {
    match reg.0 {
        20 => "ymm2",
        21 => "ymm3",
        22 => "ymm4",
        23 => "ymm5",
        24 => "ymm6",
        25 => "ymm7",
        26 => "ymm8",
        27 => "ymm9",
        28 => "ymm10",
        29 => "ymm11",
        30 => "ymm12",
        31 => "ymm13",
        32 => "ymm14",
        33 => "ymm15",
        _ => unreachable!("invalid x86 SIMD register index {}", reg.0),
    }
}

/// Map a PhysReg index to its x86-64 32-bit sub-register name.
/// Handles both callee-saved (1-5) and caller-saved (10-15) registers.
#[inline]
pub(super) fn phys_reg_name_32(reg: PhysReg) -> &'static str {
    match reg.0 {
        1 => "ebx",
        2 => "r12d",
        3 => "r13d",
        4 => "r14d",
        5 => "r15d",
        6 => "ebp",
        10 => "r11d",
        11 => "r10d",
        12 => "r8d",
        13 => "r9d",
        14 => "edi",
        15 => "esi",
        16 => "edx",
        _ => unreachable!("invalid x86 register index {}", reg.0),
    }
}

/// Get the typed register name for a PhysReg at a given IrType width.
/// E.g., PhysReg(1)=rbx → for I32 returns "ebx", for I8 returns "bl".
pub(super) fn typed_phys_reg_name(reg: PhysReg, ty: IrType) -> &'static str {
    match ty {
        IrType::I64 | IrType::U64 | IrType::Ptr => phys_reg_name(reg),
        IrType::I32 | IrType::U32 | IrType::F32 => phys_reg_name_32(reg),
        IrType::I16 | IrType::U16 => match reg.0 {
            1 => "bx",
            2 => "r12w",
            3 => "r13w",
            4 => "r14w",
            5 => "r15w",
            6 => "bp",
            10 => "r11w",
            11 => "r10w",
            12 => "r8w",
            13 => "r9w",
            14 => "di",
            15 => "si",
            16 => "dx",
            _ => phys_reg_name(reg),
        },
        IrType::I8 | IrType::U8 => match reg.0 {
            1 => "bl",
            2 => "r12b",
            3 => "r13b",
            4 => "r14b",
            5 => "r15b",
            6 => "bpl",
            10 => "r11b",
            11 => "r10b",
            12 => "r8b",
            13 => "r9b",
            14 => "dil",
            15 => "sil",
            16 => "dl",
            _ => phys_reg_name(reg),
        },
        _ => phys_reg_name(reg),
    }
}

/// Width info for an integer compare at `ty`: (cmp mnemonic, test mnemonic,
/// accumulator register). Sub-word types use their native byte/word width so a
/// narrowed comparison (`c < 0x80` on a promoted byte) emits `cmpb`/`testb`
/// instead of widening through a 32/64-bit compare — GCC/ICX's byte-compare
/// shape. I32/U32 use 32-bit ops; everything else stays 64-bit (unchanged).
pub(super) fn cmp_width_info(ty: IrType) -> (&'static str, &'static str, &'static str) {
    match ty {
        IrType::I8 | IrType::U8 => ("cmpb", "testb", "al"),
        IrType::I16 | IrType::U16 => ("cmpw", "testw", "ax"),
        IrType::I32 | IrType::U32 => ("cmpl", "testl", "eax"),
        _ => ("cmpq", "testq", "rax"),
    }
}

/// Scan inline asm instructions in a function and collect any callee-saved
/// registers that are used via specific constraints or listed in clobbers.
/// These must be saved/restored in the function prologue/epilogue.
///
/// On x86-64, the scratch register pool has 8 caller-saved GP registers
/// (rcx, rdx, rsi, rdi, r8-r11). When an inline asm block needs more than
/// 8 generic "r" operands, the scratch allocator overflows into callee-saved
/// registers (r12-r15). We use `_with_overflow` to detect this and
/// conservatively mark all callee-saved registers as clobbered only when
/// overflow is likely, avoiding unnecessary save/restore in common cases.
pub(super) fn collect_inline_asm_callee_saved_x86(func: &IrFunction, used: &mut Vec<PhysReg>) {
    fn clobber_to_phys(name: &str) -> Option<PhysReg> {
        let name = crate::backend::inline_asm::normalize_inline_asm_reg_name(name);
        match name {
            "rbx" | "ebx" | "bx" | "bl" | "bh" => Some(PhysReg(1)),
            "r12" | "r12d" | "r12w" | "r12b" => Some(PhysReg(2)),
            "r13" | "r13d" | "r13w" | "r13b" => Some(PhysReg(3)),
            "r14" | "r14d" | "r14w" | "r14b" => Some(PhysReg(4)),
            "r15" | "r15d" | "r15w" | "r15b" => Some(PhysReg(5)),
            _ => None,
        }
    }
    crate::backend::stack_layout::collect_inline_asm_callee_saved_with_overflow(
        func,
        used,
        constraint_to_callee_saved_x86,
        clobber_to_phys,
        &X86_CALLEE_SAVED,
        8,
    );
}

/// Check if a constraint string refers to a specific x86-64 callee-saved register.
fn constraint_to_callee_saved_x86(constraint: &str) -> Option<PhysReg> {
    // Handle explicit register constraint: {regname}
    if constraint.starts_with('{') && constraint.ends_with('}') {
        let reg = &constraint[1..constraint.len() - 1];
        return match reg {
            "rbx" | "ebx" | "bx" | "bl" | "bh" => Some(PhysReg(1)),
            "r12" | "r12d" | "r12w" | "r12b" => Some(PhysReg(2)),
            "r13" | "r13d" | "r13w" | "r13b" => Some(PhysReg(3)),
            "r14" | "r14d" | "r14w" | "r14b" => Some(PhysReg(4)),
            "r15" | "r15d" | "r15w" | "r15b" => Some(PhysReg(5)),
            _ => None,
        };
    }
    // Check single-character constraint letters
    // Note: 'a','c','d','S','D' are caller-saved, no save needed
    for ch in constraint.chars() {
        if ch == 'b' {
            return Some(PhysReg(1)); // rbx
        }
    }
    None
}

/// Map an ALU binary op (Add/Sub/And/Or/Xor) to its x86 mnemonic base.
pub(super) fn alu_mnemonic(op: IrBinOp) -> &'static str {
    match op {
        IrBinOp::Add => "add",
        IrBinOp::Sub => "sub",
        IrBinOp::And => "and",
        IrBinOp::Or => "or",
        IrBinOp::Xor => "xor",
        _ => unreachable!("not a simple ALU op"),
    }
}

/// Map a shift op to its x86 32-bit and 64-bit mnemonic.
pub(super) fn shift_mnemonic(op: IrBinOp) -> (&'static str, &'static str) {
    match op {
        IrBinOp::Shl => ("shll", "shlq"),
        IrBinOp::AShr => ("sarl", "sarq"),
        IrBinOp::LShr => ("shrl", "shrq"),
        _ => unreachable!("not a shift op"),
    }
}

/// x86-64 code generator. Implements the ArchCodegen trait for the shared framework.
/// Uses System V AMD64 ABI with linear scan register allocation for callee-saved registers.
pub struct X86Codegen {
    pub(crate) state: CodegenState,
    pub(super) current_return_type: IrType,
    /// SysV ABI eightbyte classification for the current function's return struct.
    /// Set in prologue, used in emit_return_i128_to_regs for the function's own return.
    pub(super) func_ret_classes: Vec<crate::common::types::EightbyteClass>,
    /// True once this function emitted an explicit `SetReturnF64Second`.
    /// For [Sse, Sse] two-eightbyte returns whose lowering routes the high
    /// half through SetReturnF64Second (IS-02), the primary I128 return must
    /// only write xmm0 — writing xmm1 from %rdx would clobber the explicit
    /// second-half value.
    pub(super) func_set_second_ret: bool,
    /// True when the current function returns _Float128 (ONE 16-byte XMM, xmm0).
    pub(super) func_ret_is_f128_sse: bool,
    /// SysV ABI eightbyte classification for the most recent call-site's return struct.
    /// Set before processing a call's result, used in emit_call_store_result.
    pub(super) call_ret_classes: Vec<crate::common::types::EightbyteClass>,
    /// True when the current call returns _Float128 (ONE 16-byte XMM, xmm0).
    pub(super) call_ret_is_f128_sse: bool,
    /// For variadic functions: number of named integer/pointer parameters (excluding long double)
    pub(super) num_named_int_params: usize,
    /// For variadic functions: number of named float/double parameters (excluding long double)
    pub(super) num_named_fp_params: usize,
    /// For variadic functions: total bytes of named parameters that are always stack-passed
    /// (e.g. long double = 16 bytes each, struct params passed by value on stack)
    pub(super) num_named_stack_bytes: usize,
    /// For variadic functions: stack offset of the register save area (negative from rbp)
    pub(super) reg_save_area_offset: i64,
    /// Whether the current function is variadic
    pub(super) is_variadic: bool,
    /// Same-block div/rem pair fusion (compute_i686_divrem_pairs with the
    /// X86_64 target): dest value-ids of TAIL instructions — they emit
    /// nothing because the HEAD's dual-store wrote their result.
    pub(super) divrem_tail_dests: FxHashSet<u32>,
    /// Head dest value-id -> (partner tail dest, partner result is the
    /// quotient in %rax). `false` means the partner wants the remainder
    /// in %rdx.
    pub(super) divrem_head_partners: FxHashMap<u32, (u32, bool)>,
    /// Tails whose pair was broken at head-emission time (pathological home
    /// combination): they must emit their own standalone division.
    pub(super) divrem_broken_tails: FxHashSet<u32>,
    /// Whether the register save area must preserve GP argument registers:
    /// true when any va_arg in the body reads INTEGER class, or the va_list
    /// escapes to a callee (which may read any class).
    pub(super) vararg_gp_save: bool,
    /// Whether the register save area must preserve SSE argument registers:
    /// true when any va_arg in the body reads SSE class, or the va_list escapes.
    pub(super) vararg_fp_save: bool,
    /// True between emit_call_stack_args_impl and emit_call_cleanup_impl
    /// when the outgoing area was dynamically realigned (a stack argument
    /// requires alignment > 16): the cleanup must restore %rsp exactly from
    /// the 16-byte save slot above the argument area instead of a static
    /// `addq` (the realignment delta is runtime-dependent).
    pub(super) dyn_align_cleanup: bool,
    /// Scratch register index for inline asm allocation (GP registers)
    pub(super) asm_scratch_idx: usize,
    /// Scratch register index for inline asm allocation (XMM registers)
    pub(super) asm_xmm_scratch_idx: usize,
    /// Register allocation results for the current function.
    /// Maps value ID -> callee-saved register assignment.
    pub(super) reg_assignments: FxHashMap<u32, PhysReg>,
    /// Which callee-saved registers are used and need save/restore.
    pub(super) used_callee_saved: Vec<PhysReg>,
    /// Whether SSE is disabled (-mno-sse). When true, variadic prologues skip
    /// XMM saves and va_start sets fp_offset to overflow immediately.
    pub(super) function_alignment: u32,
    pub(super) skip_rax_setup: bool,
    /// The current function contains a Call/CallIndirect. With frame-pointer
    /// omission a frame-less (raw_space == 0) function still needs an 8-byte
    /// pad so %rsp is 16-byte aligned at each call (entry %rsp ≡ 8 mod 16).
    /// Set by calculate_stack_space_impl, read by aligned_frame_size_impl.
    pub(super) func_has_calls: bool,
    /// FP contraction contract (-ffp-contract=fast / -ffast-math): gates the
    /// scalar mul+add -> vfmadd231 fusion.
    pub(super) fp_contract: crate::common::fp_contract::FpContract,
    /// FMA3 ISA availability (-mfma / -march=x86-64-v3+). vfmadd231s{s,d} is
    /// not in the baseline SSE2 ISA: contraction alone must not emit it.
    pub(super) fma_enabled: bool,
    pub(super) no_sse: bool,
    /// Per-function vector-constant pool: (value, byte_width) -> label.
    /// _mm256_set1_* with a constant argument lowers to a single
    /// vpbroadcast{w,d,q} .LvcN(%rip), %ymm0 (1 uop on the load port) instead
    /// of movl+vmovd+vpbroadcastd (3 uops, two of them port-5). Entries are
    /// flushed to .rodata by emit_vector_const_rodata after the function body.
    pub(super) vec_const_labels: crate::common::fx_hash::FxHashMap<(u64, u8), String>,
    pub(super) vec_const_counter: u32,
    /// True when the target has BMI1; enables scalar ANDN fusion.
    pub(super) bmi1_enabled: bool,
    /// True when the target has LZCNT/ABM (-mlzcnt or an enabling -march).
    /// When false, Clz/Ctz lower to BSR/BSF sequences with an explicit zero
    /// fixup: F3-prefixed LZCNT/TZCNT decode as BSR/BSF on CPUs without the
    /// feature and silently return wrong counts.
    pub(super) lzcnt_enabled: bool,
    /// True when the target has POPCNT (-mpopcnt or an enabling -march).
    /// When false, Popcount lowers to an shr/adc bit loop.
    pub(super) popcnt_enabled: bool,
    /// True when the target has AVX2; gates YMM constant-size copies.
    pub(super) avx2_enabled: bool,
    /// True when the target has AVX-512F; enables EVEX GPR-source broadcasts.
    pub(super) avx512_enabled: bool,
    /// -Os/-Oz: prefer the shorter sequence over the faster one (codegen-side
    /// strength reduction for constant mul/div is skipped in favour of imul/idiv).
    pub(super) optimize_for_size: bool,
    /// Current function being generated (needed for indexed addressing detection).
    pub(super) current_func: Option<*const IrFunction>,
    /// When true, skip movslq sign-extension after 32-bit ALU ops on registers
    /// for values NOT in needs_sext_values. Set when no GEPs and no widening casts.
    pub(super) skip_i32_sext: bool,
    /// Set of value IDs that need sign-extension because they're used in 64-bit
    /// contexts (Cast to I64, GEP operands). Values NOT in this set can skip movslq.
    pub(super) needs_sext_values: FxHashSet<u32>,
    /// Values whose low-32 form is PROVABLY NON-NEGATIVE because they are
    /// the result of a Clz/Ctz/Popcount on a type of at most 32 bits
    /// (PF-16b). Any 32-bit register write zeroes bits 32..63 on x86-64,
    /// and these results are <= their bit width, so bits 31..63 are all
    /// zero: ZERO-extension of the 32-bit form equals the required
    /// SIGN-extension. Widening casts of such values take the cheaper
    /// unsigned path (movl, no REX-prefixed movs) instead of cltq/movslq.
    /// Membership is per-definition — derived values (adds etc. on top of
    /// a bitop result can wrap negative) are deliberately NOT propagated.
    pub(super) bitop_nonneg_values: FxHashSet<u32>,
    /// Cmp destinations whose result is consumed ONLY by the immediately
    /// following Select/CondBranch (same block, possibly after a flag-neutral
    /// Copy chain): maps the Cmp dest to the chain-end value the consumer
    /// references. For these, the Cmp emitter skips the boolean
    /// materialization (setcc/movzbl/store) and the consumer uses the live
    /// flags directly (jcc/cmovcc).
    pub(super) fused_cmp_dests: FxHashMap<u32, u32>,
    /// Destinations of the flag-neutral forwarding Copy/Cast instructions in a
    /// fused Cmp chain. Their value is never materialized (the consumer uses
    /// the live flags), so their emitters must skip them — emitting them would
    /// read the never-written stale register (a dead, wasted instruction).
    pub(super) fused_forward_dests: FxHashSet<u32>,
    /// Cmp destinations eligible for compare-replay: the Cmp's single use is
    /// a Select or CondBranch that is NOT adjacent to the Cmp (flags would be
    /// clobbered in between), so the consumer re-emits `cmp` from the
    /// recorded operands and uses cmovcc/jcc directly instead of testing the
    /// materialized boolean (setcc/movzbl/testq chain). Keyed by Cmp dest.
    pub(super) cmp_replay: FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)>,
    /// CMP-REPLAY operand -> consumer links built with `cmp_replay` (IS-09):
    /// merged into the RA's folded_index_uses so register-homed replay
    /// operands keep their homes until the consumer re-emits the compare.
    /// Cleared after the merge; exists only to bridge the pre-RA scan and
    /// the RA invocation inside this very large prepare function.
    pub(super) cmp_replay_operand_links: FxHashMap<u32, Vec<u32>>,
    /// FP-SELECT (S05): float Cmps whose single use is a Select. The Cmp
    /// emitter skips the ucomisd/setcc boolean entirely; every select
    /// re-derives a vcmpsd/vcmpss VEX blend mask from the recorded operands
    /// and blends the FP arms (vblendvpd/vblendvps) — the gcc shape
    /// (`vcmpnltsd + vblendvpd`), killing the
    /// ucomisd/setcc/movzbl/testq/cmov + GPR-shuttle chain per select.
    /// Keyed by Cmp dest; pruned post-RA in prologue.rs (readability).
    pub(super) fp_select_cmps: FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)>,
    /// Number of uses of each SSA value (instructions + terminators).
    pub(super) value_use_counts: FxHashMap<u32, u32>,
    /// W2 Load->Cast fold (2026-08-10): load dest value id -> the register of
    /// its single consumer Cast's dest. The load emits directly into that
    /// register and the Cast emits nothing — kills the `movzbl (%p),%rX;
    /// mov %rX,%rYd` staging pair that was 33% of gzip-9 runtime cycles in
    /// longest_match's first-match-check blocks.
    pub(super) load_cast_fold: FxHashMap<u32, (PhysReg, u32)>,
    pub(super) folded_cast_dests: FxHashSet<u32>,
    /// Runtime handshake for the Load->Cast fold: set to the cast dest when a
    /// load ACTUALLY emitted the redirected register load; the adjacent Cast
    /// skips itself only when this matches (sound by construction: the cast is
    /// never skipped unless the load truly delivered the value). Cleared by
    /// every non-redirecting load and by every emitted cast.
    pub(super) fold_skip_cast: Option<u32>,
    /// Type of each SSA value (for type-aware stack slot loads/stores).
    pub(super) value_types: FxHashMap<u32, IrType>,
    /// Value ids whose 16-byte i128 slot home is written eagerly at their
    /// definition (i128 ParamRefs, 128-bit call results) — the only
    /// slot-homed i128 values the typed store route may read. See the
    /// prologue scan that fills it.
    pub(super) coherent_i128_homes: crate::common::fx_hash::FxHashSet<u32>,
    /// Pending condition flags from a fused Cmp: (cmp dest value, cmp opcode).
    /// Set by the Cmp emitter when the consumer is the next instruction.
    pub(super) pending_cmp: Option<(u32, crate::ir::reexports::IrCmpOp)>,
    /// PF-15 deferred narrowing widen (2026-08-29): a single-use widening
    /// Cast from a byte/half type whose result feeds an integer compare
    /// DEFERS its extending move and records itself here (two slots: both
    /// operands of a compare may be widening casts). The next Cmp consumes
    /// the pair and emits one `cmpb`/`cmpw` on the ORIGINAL sources --
    /// exactly GCC's shape -- instead of two extends plus a 32/64-bit
    /// compare (PF-15: `a < b` on signed chars widened both operands three
    /// times). Any other instruction flushes the deferred moves unchanged,
    /// so the transform is invisible to every non-matching consumer.
    /// Soundness rests on the record-time conflict rule in
    /// `try_record_pending_widen`: a second cast whose destination home
    /// equals the first cast's source register home is refused (the RA may
    /// reuse the first source's home for the second destination once the
    /// source's only modelled use -- the deferred move -- leaves its live
    /// range), and the reversed aliasing (first dest == second source) is
    /// impossible because the two SSA ranges overlap by construction.
    pub(super) pending_widen: [Option<super::cast_ops::PendingWiden>; 2],
    /// Pending FP condition flags from a fused float Cmp: (cmp dest value,
    /// pre-computed jcc). FP ucomisd sets EFLAGS with different semantics
    /// than integer cmp (ordered via CF/ZF/PF; `ja`/`jae`/`jb`/`jbe` map to
    /// the relational ops, NOT `jg`/`jge`/`jl`/`jle`), so the consumer must
    /// use a float-specific jcc. Set by `emit_float_cmp_impl` when the
    /// result is consumed only by an adjacent Select/CondBranch; the live
    /// flags survive to the consumer (no flag-clobbering instruction between
    /// them — same adjacency guarantee as the integer fusion).
    pub(super) pending_fp_cmp: Option<(u32, &'static str)>,
    /// IVSR pointer detection results: maps pointer phi value ID to its metadata
    pub(super) ivsr_pointers: FxHashMap<u32, IvsrPointerInfo>,
    /// Maps IVSR pointer phi to original loop counter value ID
    pub(super) pointer_to_counter: FxHashMap<u32, u32>,
    /// Tracks which callee-saved register was loaded from which ABI arg register
    /// during parameter storage. Used to avoid register-direct round-trips
    /// (e.g., rdi→rbx→rdi) that the peephole would eliminate.
    /// Maps PhysReg ID → arg register name (e.g., PhysReg(1) → "rdi").
    pub(super) param_source_regs: FxHashMap<u8, &'static str>,
    /// Phase 2b: caller-saved registers with call-spanning values.
    /// Maps PhysReg ID → dedicated spill slot for save/restore at call sites.
    pub(super) caller_save_spill_slots: FxHashMap<u8, crate::backend::state::StackSlot>,
    /// Phase 2b: live intervals for each caller-saved-spanning register.
    /// Maps PhysReg ID → list of (start, end) program points.
    /// At each call, only save registers with an interval containing the call point.
    pub(super) caller_save_intervals: FxHashMap<u8, Vec<(u32, u32)>>,
    /// MachInst buffer for virtual register ISel. Instructions are accumulated
    /// here by try_lower_machinst and flushed (allocated + emitted) by flush_machinst.
    pub(super) machinst_buf: Vec<super::machinst::MachInst>,
    /// Shadow of the IR instructions currently in `machinst_buf`, in the same
    /// order. Used by the fallback path: if the MachInst buffer cannot be fully
    /// resolved (a value on the stack in a register-only position), the
    /// buffered IR instructions are re-emitted through the default codegen
    /// path instead of being silently deleted (which miscompiled gzip:
    /// missing loads/ALU -> segfault).
    pub(super) machinst_buf_ir: Vec<crate::ir::reexports::Instruction>,
    /// Whether MachInst ISel is globally enabled. Default-on after gzip and
    /// SQLite correctness bring-up; CCC_NO_MACHINST=1 is the fail-safe switch.
    pub(super) machinst_enabled: bool,
    /// Per-function profitability decision. Large loop bodies stay on the
    /// mature backend until MachInst has loop-aware scheduling.
    pub(super) machinst_function_enabled: bool,
    /// Diagnostic/profitability mask from CCC_MI_DISABLE_KINDS. Each bit
    /// disables one IR instruction family without disabling whole functions.
    pub(super) machinst_disabled_kinds: u32,
}

const MI_BINOP: u32 = 1 << 0;
const MI_LOAD: u32 = 1 << 1;
const MI_STORE: u32 = 1 << 2;
const MI_COPY: u32 = 1 << 3;
const MI_CMP: u32 = 1 << 4;
const MI_CAST: u32 = 1 << 5;
const MI_UNARY: u32 = 1 << 6;
const MI_SELECT: u32 = 1 << 7;
const MI_GEP: u32 = 1 << 8;
const MI_ALLOCA: u32 = 1 << 9;
const MI_CAST_WIDEN: u32 = 1 << 10;
const MI_CAST_NARROW: u32 = 1 << 11;
const MI_CAST_SAME: u32 = 1 << 12;
const MI_CAST_SIGNED_SOURCE: u32 = 1 << 13;
const MI_CAST_UNSIGNED_SOURCE: u32 = 1 << 14;
/// Scalar float moves (FMov): float Store/Load/Copy lowered to movss/movsd.
/// Separate bit so float lowering can be bisected without losing integer
/// coverage of the same instruction kinds.
const MI_FLOAT_MOV: u32 = 1 << 15;
/// Scalar float arithmetic (FAlu): float Add/Sub/Mul/Div lowered to the VEX
/// three-operand vadd/vsub/vmul/vdiv forms.
const MI_FLOAT_ALU: u32 = 1 << 16;
/// Typed direct calls (CallTyped): SysV integer-register calls lowered as one
/// atomic machine instruction (caller-save spills + argument moves + call +
/// return home + restores), replacing the accumulator-staged text sequence.
const MI_CALL_TYPED: u32 = 1 << 17;

fn parse_machinst_disabled_kinds() -> u32 {
    std::env::var("CCC_MI_DISABLE_KINDS")
        .unwrap_or_default()
        .split(',')
        .fold(0, |mask, name| {
            mask | match name.trim() {
                "binop" => MI_BINOP,
                "load" => MI_LOAD,
                "store" => MI_STORE,
                "copy" => MI_COPY,
                "cmp" => MI_CMP,
                "cast" => MI_CAST,
                "unary" => MI_UNARY,
                "select" => MI_SELECT,
                "gep" => MI_GEP,
                "alloca" => MI_ALLOCA,
                "cast-widen" => MI_CAST_WIDEN,
                "cast-narrow" => MI_CAST_NARROW,
                "cast-same" => MI_CAST_SAME,
                "cast-signed-source" => MI_CAST_SIGNED_SOURCE,
                "cast-unsigned-source" => MI_CAST_UNSIGNED_SOURCE,
                "float-mov" => MI_FLOAT_MOV,
                "float-alu" => MI_FLOAT_ALU,
                "call-typed" => MI_CALL_TYPED,
                "all" => u32::MAX,
                _ => 0,
            }
        })
}

fn machinst_kind_bit(inst: &crate::ir::reexports::Instruction) -> u32 {
    use crate::ir::reexports::Instruction;
    match inst {
        Instruction::BinOp { .. } => MI_BINOP,
        Instruction::Load { .. } => MI_LOAD,
        Instruction::Store { .. } => MI_STORE,
        Instruction::Copy { .. } => MI_COPY,
        Instruction::Cmp { .. } => MI_CMP,
        Instruction::Cast { .. } => MI_CAST,
        Instruction::UnaryOp { .. } => MI_UNARY,
        Instruction::Select { .. } => MI_SELECT,
        Instruction::GetElementPtr { .. } => MI_GEP,
        Instruction::Alloca { .. } => MI_ALLOCA,
        Instruction::Call { .. } | Instruction::CallIndirect { .. } => MI_CALL_TYPED,
        _ => 0,
    }
}

/// Information about an IVSR-transformed pointer induction variable
#[derive(Debug, Clone)]
pub(super) struct IvsrPointerInfo {
    pub phi_val: Value,          // The pointer phi node
    pub base_ptr: Value,         // Original array base
    pub stride: i64,             // Element size in bytes
    pub init_offset: i64,        // Initial offset (usually 0)
    pub header_block: BlockId,   // Loop header containing phi
    pub backedge_block: BlockId, // Block with pointer increment
}

impl X86Codegen {
    /// Compare the low 64-bit half of a 128-bit switch value (already in
    /// %rax) against `case_val` and branch to `label` on equality.  The sign
    /// class of the case has already been verified against the high half.
    fn emit_switch_case_branch_128_low(&mut self, case_val: i64, label: &str) {
        if case_val == 0 {
            self.state.emit("    testq %rax, %rax");
        } else if case_val >= i32::MIN as i64 && case_val <= i32::MAX as i64 {
            self.state
                .out
                .emit_instr_imm_reg("    cmpq", case_val, "rax");
        } else {
            self.state
                .out
                .emit_instr_imm_reg("    movabsq", case_val, "rcx");
            self.state.emit("    cmpq %rcx, %rax");
        }
        self.state.out.emit_jcc_label("    je", label);
    }

    /// Density-based switch lowering for native-width values: the trait
    /// default's logic (jump table with PGO case hoisting, or a sparse
    /// compare chain), reproduced here so the 128-bit path can coexist in
    /// the override above.
    fn emit_switch_native(
        &mut self,
        val: &Operand,
        cases: &[(i64, BlockId)],
        default: &BlockId,
        ty: IrType,
    ) {
        let hint = crate::pgo::take_switch_hint();
        let force_chain = hint.map(|h| h.force_chain).unwrap_or(false);
        let use_jump_table = if self.state_ref().no_jump_tables || force_chain {
            false
        } else if cases.len() >= MIN_JUMP_TABLE_CASES {
            let min_val = cases
                .iter()
                .map(|&(v, _)| v)
                .min()
                .expect("switch must have cases");
            let max_val = cases
                .iter()
                .map(|&(v, _)| v)
                .max()
                .expect("switch must have cases");
            let range_i128 = max_val as i128 - min_val as i128 + 1;
            let range_ok = range_i128 > 0 && range_i128 <= MAX_JUMP_TABLE_RANGE as i128;
            range_ok && cases.len() * 100 / (range_i128 as usize) >= MIN_JUMP_TABLE_DENSITY_PERCENT
        } else {
            false
        };

        if use_jump_table {
            let mut cases2: Vec<(i64, BlockId)> = cases.to_vec();
            let mut hoist: Option<(i64, BlockId)> = None;
            if let Some(h) = hint {
                if let Some((hv, ht)) = h.hot_case {
                    if let Some(pos) = cases2.iter().position(|&(v, t)| v == hv && t.0 == ht) {
                        hoist = Some(cases2.remove(pos));
                    }
                }
            }
            if let Some((hv, ht)) = hoist {
                self.emit_load_operand(val);
                self.emit_switch_case_branch(hv, &ht.as_label(), ty);
            }
            self.emit_switch_jump_table(val, &cases2, default, ty);
        } else {
            self.emit_load_operand(val);
            for &(case_val, target) in cases {
                let label = target.as_label();
                self.emit_switch_case_branch(case_val, &label, ty);
            }
            self.emit_branch_to_block(*default);
        }
    }

    /// Emit a retpoline-protected indirect CALL through `reg`.
    ///
    /// thunk-inline (kernel vDSO: userspace code that cannot reference the
    /// kernel's __x86_indirect_thunk_* symbols) expands the full retpoline
    /// at the call site — byte-equivalent to GCC's
    /// -mindirect-branch=thunk-inline output (verified against gcc -O2 on
    /// the fp-call reproducer):
    ///     jmp .Louter
    ///   .Linner: call .Lset
    ///   .Lspec:  pause; lfence; jmp .Lspec   ; speculation trap
    ///   .Lset:   mov %reg,(%rsp); ret        ; RSB-safe dispatch
    ///   .Louter: call .Linner                ; pushes the REAL return addr
    /// thunk-extern emits `call __x86_indirect_thunk_<reg>` (kernel proper,
    /// resolved at link time and patchable by objtool --retpoline).
    pub(super) fn emit_retpoline_call(&mut self, reg: &str) {
        if self.state.indirect_branch_thunk_inline {
            let id = self.state.next_label_id();
            self.state
                .emit_fmt(format_args!("    jmp .Lrpl_outer_{}", id));
            self.state.emit_fmt(format_args!(".Lrpl_inner_{}:", id));
            self.state
                .emit_fmt(format_args!("    call .Lrpl_set_{}", id));
            self.state.emit_fmt(format_args!(".Lrpl_spec_{}:", id));
            self.state.emit("    pause");
            self.state.emit("    lfence");
            self.state
                .emit_fmt(format_args!("    jmp .Lrpl_spec_{}", id));
            self.state.emit_fmt(format_args!(".Lrpl_set_{}:", id));
            self.state
                .emit_fmt(format_args!("    movq %{}, (%rsp)", reg));
            self.state.emit("    ret");
            self.state.emit_fmt(format_args!(".Lrpl_outer_{}:", id));
            self.state
                .emit_fmt(format_args!("    call .Lrpl_inner_{}", id));
        } else if self.state.indirect_branch_thunk {
            self.state
                .emit_fmt(format_args!("    call __x86_indirect_thunk_{}", reg));
        } else {
            self.state.emit_fmt(format_args!("    call *%{}", reg));
        }
    }

    /// Emit a retpoline-protected indirect JMP through `reg` (tail calls,
    /// computed goto, switch tables). GCC's jump form:
    ///     call .Lset          ; pushes .Lspec as the "return" address
    ///   .Lspec: pause; lfence; jmp .Lspec
    ///   .Lset:  mov %reg,(%rsp); ret   ; ret consumes the rewritten slot
    pub(super) fn emit_retpoline_jump(&mut self, reg: &str) {
        if self.state.indirect_branch_thunk_inline {
            let id = self.state.next_label_id();
            self.state
                .emit_fmt(format_args!("    call .Lrpl_set_{}", id));
            self.state.emit_fmt(format_args!(".Lrpl_spec_{}:", id));
            self.state.emit("    pause");
            self.state.emit("    lfence");
            self.state
                .emit_fmt(format_args!("    jmp .Lrpl_spec_{}", id));
            self.state.emit_fmt(format_args!(".Lrpl_set_{}:", id));
            self.state
                .emit_fmt(format_args!("    movq %{}, (%rsp)", reg));
            self.state.emit("    ret");
        } else if self.state.indirect_branch_thunk {
            self.state
                .emit_fmt(format_args!("    jmp __x86_indirect_thunk_{}", reg));
        } else {
            self.state.emit_fmt(format_args!("    jmpq *%{}", reg));
        }
    }

    pub fn new() -> Self {
        Self {
            state: CodegenState::new(),
            current_return_type: IrType::I64,
            func_ret_classes: Vec::new(),
            func_set_second_ret: false,
            call_ret_is_f128_sse: false,
            call_ret_classes: Vec::new(),
            func_ret_is_f128_sse: false,
            num_named_int_params: 0,
            num_named_fp_params: 0,
            num_named_stack_bytes: 0,
            reg_save_area_offset: 0,
            is_variadic: false,
            divrem_tail_dests: FxHashSet::default(),
            divrem_head_partners: FxHashMap::default(),
            divrem_broken_tails: FxHashSet::default(),
            vararg_gp_save: true,
            vararg_fp_save: true,
            dyn_align_cleanup: false,
            asm_scratch_idx: 0,
            asm_xmm_scratch_idx: 0,
            reg_assignments: FxHashMap::default(),
            used_callee_saved: Vec::new(),
            function_alignment: 0,
            skip_rax_setup: false,
            func_has_calls: false,
            fp_contract: crate::common::fp_contract::FpContract::default(),
            fma_enabled: false,
            no_sse: false,
            vec_const_labels: crate::common::fx_hash::FxHashMap::default(),
            vec_const_counter: 0,
            bmi1_enabled: false,
            lzcnt_enabled: false,
            popcnt_enabled: false,
            avx2_enabled: false,
            avx512_enabled: false,
            optimize_for_size: false,
            skip_i32_sext: false,
            needs_sext_values: FxHashSet::default(),
            bitop_nonneg_values: FxHashSet::default(),
            fused_cmp_dests: FxHashMap::default(),
            fused_forward_dests: FxHashSet::default(),
            cmp_replay: FxHashMap::default(),
            cmp_replay_operand_links: FxHashMap::default(),
            fp_select_cmps: FxHashMap::default(),
            value_use_counts: FxHashMap::default(),
            load_cast_fold: FxHashMap::default(),
            folded_cast_dests: FxHashSet::default(),
            fold_skip_cast: None,
            value_types: FxHashMap::default(),
            coherent_i128_homes: crate::common::fx_hash::FxHashSet::default(),
            pending_cmp: None,
            pending_widen: [None, None],
            pending_fp_cmp: None,
            current_func: None,
            ivsr_pointers: FxHashMap::default(),
            pointer_to_counter: FxHashMap::default(),
            param_source_regs: FxHashMap::default(),
            caller_save_spill_slots: FxHashMap::default(),
            caller_save_intervals: FxHashMap::default(),
            machinst_buf: Vec::new(),
            machinst_buf_ir: Vec::new(),
            machinst_enabled: std::env::var("CCC_NO_MACHINST").is_err(),
            machinst_function_enabled: false,
            machinst_disabled_kinds: parse_machinst_disabled_kinds(),
        }
    }

    /// Enable position-independent code generation.
    pub fn set_pic(&mut self, pic: bool) {
        self.state.pic_mode = pic;
    }

    /// Enable function return thunk (-mfunction-return=thunk-extern).
    pub fn set_function_return_thunk(&mut self, enabled: bool) {
        self.state.function_return_thunk = enabled;
    }

    /// Enable indirect branch thunk (-mindirect-branch=thunk-extern).
    pub fn set_indirect_branch_thunk(&mut self, enabled: bool) {
        self.state.indirect_branch_thunk = enabled;
    }

    /// Enable inline retpoline expansion (-mindirect-branch=thunk-inline).
    pub fn set_indirect_branch_thunk_inline(&mut self, enabled: bool) {
        self.state.indirect_branch_thunk_inline = enabled;
    }

    /// Set patchable function entry configuration (-fpatchable-function-entry=N,M).
    pub fn set_patchable_function_entry(&mut self, entry: Option<(u32, u32)>) {
        self.state.patchable_function_entry = entry;
    }

    /// Set function-entry mcount instrumentation (-pg / -mfentry /
    /// -mrecord-mcount / -mnop-mcount). When set, codegen emits a 5-byte call
    /// (or NOP) at function entry, optionally recorded in __mcount_loc.
    pub fn set_mcount(&mut self, m: Option<crate::backend::McountInstrumentation>) {
        self.state.mcount = m;
    }

    /// Enable CF protection branch (-fcf-protection=branch) to emit endbr64.
    pub fn set_cf_protection_branch(&mut self, enabled: bool) {
        self.state.cf_protection_branch = enabled;
    }

    /// Enable kernel code model (-mcmodel=kernel). All symbols are assumed
    /// to be in the negative 2GB of the virtual address space.
    pub fn set_code_model_kernel(&mut self, enabled: bool) {
        self.state.code_model_kernel = enabled;
    }

    /// Disable jump table emission (-fno-jump-tables). All switch statements
    /// use compare-and-branch chains instead of indirect jumps.
    pub fn set_no_jump_tables(&mut self, enabled: bool) {
        self.state.no_jump_tables = enabled;
    }

    /// Disable SSE (-mno-sse). Prevents emission of any SSE/XMM instructions.
    pub fn set_no_sse(&mut self, enabled: bool) {
        self.no_sse = enabled;
    }

    /// Apply all relevant options from a `CodegenOptions` struct.
    pub fn apply_options(&mut self, opts: &crate::backend::CodegenOptions) {
        self.state.disable_regalloc = opts.disable_regalloc;
        self.set_pic(opts.pic || opts.pie);
        self.state.pie_mode = opts.pie;
        self.set_function_return_thunk(opts.function_return_thunk);
        self.set_indirect_branch_thunk(opts.indirect_branch_thunk);
        self.set_indirect_branch_thunk_inline(opts.indirect_branch_thunk_inline);
        self.set_patchable_function_entry(opts.patchable_function_entry);
        self.set_mcount(opts.mcount);
        self.set_cf_protection_branch(opts.cf_protection_branch);
        self.function_alignment = opts.function_alignment;
        self.skip_rax_setup = opts.skip_rax_setup;
        self.set_no_sse(opts.no_sse);
        self.set_code_model_kernel(opts.code_model_kernel);
        self.set_no_jump_tables(opts.no_jump_tables);
        self.bmi1_enabled = opts.bmi1;
        self.lzcnt_enabled = opts.lzcnt;
        self.popcnt_enabled = opts.popcnt;
        self.avx2_enabled = opts.avx2;
        self.avx512_enabled = opts.avx512;
        self.state.emit_cfi = opts.emit_cfi;
        self.fp_contract = opts.fp_contract;
        self.fma_enabled = opts.fma;
        self.optimize_for_size = opts.optimize_for_size;
    }

    /// Reserve (or reuse) a .rodata label holding `value` as `width` bytes for a
    /// vpbroadcast* splat, and return "label(%rip)" for the RIP-relative operand.
    pub(super) fn vec_const_rip_operand(&mut self, value: u64, width: u8) -> String {
        let label = match self.vec_const_labels.get(&(value, width)) {
            Some(l) => l.clone(),
            None => {
                let id = self.vec_const_counter;
                self.vec_const_counter += 1;
                let l = format!(".Lvc{}", id);
                self.vec_const_labels.insert((value, width), l.clone());
                l
            }
        };
        format!("{}(%rip)", label)
    }

    /// Emit the per-function vector-constant pool entries to .rodata.
    pub(super) fn emit_vector_const_rodata_impl(&mut self) {
        if self.vec_const_labels.is_empty() {
            return;
        }
        let sect = self.state.current_text_section.clone();
        self.state.emit(".section .rodata");
        self.state.emit(".align 4");
        let mut entries: Vec<(&(u64, u8), &String)> = self.vec_const_labels.iter().collect();
        entries.sort_by_key(|(k, _)| (k.1, k.0));
        for ((value, width), label) in entries {
            self.state.out.emit_named_label(label);
            match width {
                1 => self
                    .state
                    .emit_fmt(format_args!("    .byte {}", *value as u8)),
                2 => self
                    .state
                    .emit_fmt(format_args!("    .short {}", *value as u16)),
                8 => self
                    .state
                    .emit_fmt(format_args!("    .quad {}", *value as i64)),
                _ => self
                    .state
                    .emit_fmt(format_args!("    .long {}", *value as i32)),
            }
        }
        self.state
            .emit_fmt(format_args!(".section {},\"ax\",@progbits", sect));
        // Keep the counter monotonic across functions so every .LvcN label is
        // unique module-wide (dedup only happens within one function's pool,
        // which is flushed together). Resetting per function would emit
        // duplicate .rodata labels that resolve to the wrong width.
        self.vec_const_labels.clear();
    }

    /// Format a stack slot reference as "offset(%rbp)" or "offset(%rsp)" depending
    /// on whether the frame pointer is omitted. Used by format! strings throughout
    /// the codegen that need to reference stack slots directly.
    pub(super) fn slot_ref(&self, rbp_offset: i64) -> String {
        if self.state.out.use_rsp_addressing {
            format!("{}(%rsp)", self.state.out.rsp_frame_size + rbp_offset)
        } else {
            format!("{}(%rbp)", rbp_offset)
        }
    }

    // --- x86 helper methods ---

    /// Get the callee-saved register assigned to an operand, if any.
    pub(super) fn operand_reg(&self, op: &Operand) -> Option<PhysReg> {
        match op {
            Operand::Value(v) => self.reg_assignments.get(&v.0).copied(),
            _ => None,
        }
    }

    /// Get the callee-saved register assigned to a destination value, if any.
    pub(super) fn dest_reg(&self, dest: &Value) -> Option<PhysReg> {
        self.reg_assignments.get(&dest.0).copied()
    }

    /// Physical registers safe to use directly as an addressing base for
    /// vector loads/stores: the register-allocatable GPR set MINUS
    /// rdi/rsi/rdx (PhysReg 14/15/16), which the vector-intrinsic handlers use
    /// as scratch (FmaF64x4/FmaF64x4SIB, movnt, crc32, ...) and may clobber
    /// mid-live-range even though the allocator sees them as non-call-spanning.
    /// r11/r10/r8/r9/rbx/r12-r15 are never touched by intrinsic scratch.
    pub(super) const VEC_BASE_SAFE_REGS: [u8; 9] = [1, 2, 3, 4, 5, 10, 11, 12, 13];

    /// If `val_id`'s ADDRESS can be used directly as a memory operand for a
    /// vector access, return the operand string:
    /// - register-allocated GPR pointer  -> "(%r10)"   (no movq-to-rax round trip)
    /// - alloca with a direct slot       -> "slot(%rbp)"  (data IS at the slot)
    /// Returns None for over-aligned allocas (runtime-alignment dance needed),
    /// indirect temp pointers (slot holds a pointer), XMM-assigned values,
    /// and constants.
    pub(super) fn value_ptr_mem_operand(&self, val_id: u32) -> Option<String> {
        use crate::backend::state::SlotAddr;
        if let Some(&reg) = self.reg_assignments.get(&val_id) {
            if !is_xmm_reg(reg) && Self::VEC_BASE_SAFE_REGS.contains(&reg.0) {
                let name = phys_reg_name(reg);
                return Some(format!("(%{})", name));
            }
            return None;
        }
        if let Some(addr) = self.state.resolve_slot_addr(val_id) {
            if let SlotAddr::Direct(slot) = addr {
                return Some(self.slot_ref(slot.0));
            }
            return None;
        }
        None
    }

    /// `value_ptr_mem_operand` for an Operand.
    pub(super) fn operand_ptr_mem_operand(&self, op: &Operand) -> Option<String> {
        match op {
            Operand::Value(v) => self.value_ptr_mem_operand(v.0),
            _ => None,
        }
    }

    /// CCC_ENABLE_VECREG: at the end of a block, flush every register-held
    /// vector value that is LIVE OUT (used outside this block) to its home
    /// slot, so any consumer that reads it through memory (memcpy src, call
    /// arg, alias store, sret copy) sees the data. The register still holds
    /// the value afterwards (nothing clobbers the xmm3-xmm7 pool except the
    /// value's own producer), so `vec_live_regs` is intentionally KEPT.
    pub(super) fn flush_vecreg_liveout_impl(&mut self) {
        if self.state.vec_live_regs.is_empty() {
            return;
        }
        let total = &self.state.value_use_counts;
        let block = &self.state.block_use_counts;
        let mut flush: Vec<(u32, &'static str)> = Vec::new();
        for (&v, &reg_name) in &self.state.vec_live_regs {
            let total_uses = total.get(v as usize).copied().unwrap_or(0);
            let block_uses = block.get(&v).copied().unwrap_or(0);
            if block_uses < total_uses {
                flush.push((v, reg_name));
            }
        }
        for (v, reg_name) in flush {
            // Compute the alloca's effective address directly (leaq, plus the
            // runtime-alignment dance for over-aligned slots). Must NOT go
            // through value_to_reg: for an XMM-assigned value that would load
            // the vector DATA (`movq %xmmN, %rax`) instead of the address.
            if let Some(slot) = self.state.get_slot(v) {
                if let Some(align) = self.state.alloca_over_align(v) {
                    self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rax");
                    self.state
                        .out
                        .emit_instr_imm_reg("    addq", (align - 1) as i64, "rax");
                    self.state
                        .out
                        .emit_instr_imm_reg("    andq", -(align as i64), "rax");
                } else {
                    self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rax");
                }
                // Width-correct store: vec_live_regs can hold xmm (128-bit),
                // ymm (256-bit) or zmm (512-bit) names; movdqu is the SSE
                // 128-bit form and is ILLEGAL for the wider registers (the
                // internal assembler rightly rejected `movdqu %zmm0, (%rax)`
                // in zlib-ng's adler32_avx512).
                let mov = if reg_name.starts_with("zmm") {
                    "vmovdqu64"
                } else if reg_name.starts_with("ymm") {
                    "vmovdqu"
                } else {
                    "movdqu"
                };
                self.state
                    .emit_fmt(format_args!("    {} %{}, (%rax)", mov, reg_name));
            }
        }
    }

    /// Emit sign-extension from 32-bit to 64-bit register if the type is signed.
    /// Used after 32-bit ALU operations on callee-saved registers.
    pub(super) fn emit_sext32_if_needed(
        &mut self,
        name_32: &str,
        name_64: &str,
        is_unsigned: bool,
    ) {
        if !is_unsigned {
            self.state
                .out
                .emit_instr_reg_reg("    movslq", name_32, name_64);
        }
    }

    /// Like emit_sext32_if_needed but skips movslq when the destination value
    /// has no 64-bit uses (not in needs_sext_values).
    pub(super) fn emit_sext32_for_value(
        &mut self,
        name_32: &str,
        name_64: &str,
        is_unsigned: bool,
        dest_id: u32,
    ) {
        if is_unsigned {
            return;
        }
        // Per-value check: if we computed the set and this value isn't in it, skip
        if !self.needs_sext_values.is_empty() && !self.needs_sext_values.contains(&dest_id) {
            return;
        }
        // Function-level fallback: skip if no values need sext at all
        if self.skip_i32_sext {
            return;
        }
        self.state
            .out
            .emit_instr_reg_reg("    movslq", name_32, name_64);
    }

    /// Emit a comparison instruction, optionally using 32-bit form for I32/U32 types.
    /// When `use_32bit` is true, emits `cmpl` with 32-bit register names instead of `cmpq`.
    /// COMPARE-REPLAY re-emission: like emit_int_cmp_insn_typed, but loads
    /// both operands from their CANONICAL STACK SLOTS instead of trusting
    /// register assignments. The replay runs at the consumer (Select /
    /// CondBranch) position, after intervening instructions: a register that
    /// held the operand at the original Cmp may since have been reassigned to
    /// a later-defined value (the register allocator sized live ranges
    /// against the original Cmp position). Slots are written for every
    /// non-register-allocated value, and the replay candidates are filtered
    /// to slot-resident operands at build time — so the slot is always the
    /// correct, stable source here.
    pub(super) fn emit_int_cmp_replay_insn(&mut self, lhs: &Operand, rhs: &Operand, ty: IrType) {
        let (cmp_instr, _test_instr, acc_reg) = cmp_width_info(ty);
        let use_32bit = matches!(ty, IrType::I32 | IrType::U32);
        // Load lhs from its slot with the TYPE-AWARE width. A raw `movq` slot
        // load is unsound for narrow values: the slot stores are 8-byte
        // (movq) but only the low N bytes hold the value; the upper bytes are
        // stale (slot reuse / sign-extension residue). value_to_reg emits
        // movzbl/movzwl/movsbq/movswq/movslq/movl/movq matched to the value
        // type, exactly like every other load path — comparing a U16 stateno
        // (sqlite3 yy_shift `state > 599 ? state+415 : state`) with 64-bit
        // garbage would branch wrongly and corrupt the parser stack.
        match lhs {
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                self.value_to_reg(v, "rax");
            }
            _ => {
                self.operand_to_rax(lhs);
            }
        }
        // Prefer the immediate form whenever rhs is an encodable constant,
        // matching emit_int_cmp_insn_typed (cmp $imm, %al/%eax/%rax).
        if let Some(imm) = Self::const_as_imm32_typed(rhs, use_32bit) {
            self.state
                .emit_fmt(format_args!("    {} ${}, %{}", cmp_instr, imm, acc_reg));
            return;
        }
        match rhs {
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                self.value_to_reg(v, "rcx");
            }
            _ => {
                self.operand_to_rcx(rhs);
            }
        }
        let rreg = if use_32bit { "ecx" } else { "rcx" };
        self.state
            .emit_fmt(format_args!("    {} %{}, %{}", cmp_instr, rreg, acc_reg));
    }

    pub(super) fn emit_int_cmp_insn_typed(&mut self, lhs: &Operand, rhs: &Operand, ty: IrType) {
        let (cmp_instr, test_instr, acc_reg) = cmp_width_info(ty);
        let use_32bit = matches!(ty, IrType::I32 | IrType::U32);
        // An INTEGER compare of a value homed in an XMM register (bit-punned
        // float words: GET_FLOAT_WORD in glibc's flt-32 kernels after the
        // hidden_proto wrappers inline) has no typed GPR sub-register name —
        // typed_phys_reg_name would hit unreachable!("invalid x86 register
        // index"). Route XMM-homed operands through the accumulator
        // fallbacks below, whose operand_to_rax already handles the
        // xmm→GPR move.
        let lhs_phys = self.operand_reg(lhs).filter(|r| !is_xmm_reg(*r));
        let rhs_phys = self.operand_reg(rhs).filter(|r| !is_xmm_reg(*r));
        // Prefer the immediate form whenever rhs is an encodable constant —
        // BEFORE checking register pairs, so `mask == 0xFFFFFFFF` becomes
        // `cmpl $-1, %eax` instead of movabs+cmpq.
        if rhs_phys.is_none() || matches!(rhs, Operand::Const(_)) {
            if let Some(imm) = Self::const_as_imm32_typed(rhs, use_32bit) {
                if imm == 0 {
                    if let Some(lhs_r) = lhs_phys {
                        let lhs_name = typed_phys_reg_name(lhs_r, ty);
                        self.state.emit_fmt(format_args!(
                            "    {} %{}, %{}",
                            test_instr, lhs_name, lhs_name
                        ));
                    } else {
                        self.operand_to_rax(lhs);
                        self.state.emit_fmt(format_args!(
                            "    {} %{}, %{}",
                            test_instr, acc_reg, acc_reg
                        ));
                    }
                } else if let Some(lhs_r) = lhs_phys {
                    let lhs_name = typed_phys_reg_name(lhs_r, ty);
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %{}", cmp_instr, imm, lhs_name));
                } else {
                    self.operand_to_rax(lhs);
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %{}", cmp_instr, imm, acc_reg));
                }
                return;
            }
        }
        if let (Some(lhs_r), Some(rhs_r)) = (lhs_phys, rhs_phys) {
            // Both in registers: compare directly
            let lhs_name = typed_phys_reg_name(lhs_r, ty);
            let rhs_name = typed_phys_reg_name(rhs_r, ty);
            self.state.emit_fmt(format_args!(
                "    {} %{}, %{}",
                cmp_instr, rhs_name, lhs_name
            ));
        } else if let Some(lhs_r) = lhs_phys {
            let lhs_name = typed_phys_reg_name(lhs_r, ty);
            // Memory-operand: cmp N(%rsp), %lhs_reg directly
            if let Operand::Value(rv) = rhs {
                if self.dest_reg(rv).is_none() {
                    if let Some(slot) = self.state.get_slot(rv.0) {
                        if !self.state.is_alloca(rv.0) {
                            let sref = self.slot_ref(slot.0);
                            self.state.emit_fmt(format_args!(
                                "    {} {}, %{}",
                                cmp_instr, sref, lhs_name
                            ));
                            return;
                        }
                    }
                }
            }
            self.operand_to_rcx(rhs);
            let rcx = if use_32bit { "ecx" } else { "rcx" };
            self.state
                .emit_fmt(format_args!("    {} %{}, %{}", cmp_instr, rcx, lhs_name));
        } else if let Some(rhs_r) = rhs_phys {
            let rhs_name = typed_phys_reg_name(rhs_r, ty);
            self.operand_to_rax(lhs);
            self.state.emit_fmt(format_args!(
                "    {} %{}, %{}",
                cmp_instr, rhs_name, acc_reg
            ));
        } else {
            // Neither operand has a register. Load lhs to %rax.
            self.operand_to_rax(lhs);
            // Memory-operand: cmp N(%rsp), %rax directly for rhs
            if let Operand::Value(rv) = rhs {
                if let Some(slot) = self.state.get_slot(rv.0) {
                    if !self.state.is_alloca(rv.0) {
                        let sref = self.slot_ref(slot.0);
                        self.state
                            .emit_fmt(format_args!("    {} {}, %{}", cmp_instr, sref, acc_reg));
                        return;
                    }
                }
            }
            self.operand_to_rcx(rhs);
            let rcx = if use_32bit { "ecx" } else { "rcx" };
            self.state
                .emit_fmt(format_args!("    {} %{}, %{}", cmp_instr, rcx, acc_reg));
        }
    }

    /// Handles constants, register-allocated values, and stack values.
    pub(super) fn operand_to_callee_reg(&mut self, op: &Operand, target: PhysReg) {
        let target_name = phys_reg_name(target);
        match op {
            Operand::Const(c) => {
                match c {
                    IrConst::I8(v) if *v == 0 => self.state.emit_fmt(format_args!(
                        "    xorl %{0}, %{0}",
                        phys_reg_name_32(target)
                    )),
                    IrConst::I16(v) if *v == 0 => self.state.emit_fmt(format_args!(
                        "    xorl %{0}, %{0}",
                        phys_reg_name_32(target)
                    )),
                    IrConst::I32(v) if *v == 0 => self.state.emit_fmt(format_args!(
                        "    xorl %{0}, %{0}",
                        phys_reg_name_32(target)
                    )),
                    IrConst::I64(0) => self.state.emit_fmt(format_args!(
                        "    xorl %{0}, %{0}",
                        phys_reg_name_32(target)
                    )),
                    // PF-16: non-negative immediates write the 32-bit name
                    // with movl — the 32-bit write zero-extends to the same
                    // 64-bit value a `movq $imm32` sign-extension would
                    // produce, saving the REX prefix and two bytes. Negative
                    // immediates sign-extend; movl would zero-extend instead
                    // — they keep the q-form.
                    IrConst::I8(v) => {
                        let x = *v as i64;
                        if x >= 0 {
                            self.state.emit_fmt(format_args!(
                                "    movl ${}, %{}",
                                x,
                                phys_reg_name_32(target)
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    movq ${}, %{}", x, target_name));
                        }
                    }
                    IrConst::I16(v) => {
                        let x = *v as i64;
                        if x >= 0 {
                            self.state.emit_fmt(format_args!(
                                "    movl ${}, %{}",
                                x,
                                phys_reg_name_32(target)
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    movq ${}, %{}", x, target_name));
                        }
                    }
                    IrConst::I32(v) => {
                        let x = *v as i64;
                        if x >= 0 {
                            self.state.emit_fmt(format_args!(
                                "    movl ${}, %{}",
                                x,
                                phys_reg_name_32(target)
                            ));
                        } else {
                            self.state
                                .emit_fmt(format_args!("    movq ${}, %{}", x, target_name));
                        }
                    }
                    IrConst::I64(v) => {
                        if *v >= 0 && *v <= i32::MAX as i64 {
                            // PF-16: fits and non-negative — 5-byte movl,
                            // zero-extension equals the sign-extended q-form.
                            self.state.out.emit_instr_imm_reg(
                                "    movl",
                                *v,
                                phys_reg_name_32(target),
                            );
                        } else if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movq", *v, target_name);
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", *v, target_name);
                        }
                    }
                    IrConst::Zero => self.state.emit_fmt(format_args!(
                        "    xorl %{0}, %{0}",
                        phys_reg_name_32(target)
                    )),
                    _ => {
                        // For float/i128 constants, fall back to loading to rax and moving
                        self.operand_to_rax(op);
                        self.state
                            .out
                            .emit_instr_reg_reg("    movq", "rax", target_name);
                    }
                }
            }
            Operand::Value(v) => {
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if reg.0 != target.0 {
                        if is_xmm_reg(reg) {
                            // XMM → GPR
                            let xmm_name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    movq %{}, %{}", xmm_name, target_name));
                        } else {
                            let src_name = phys_reg_name(reg);
                            self.state
                                .out
                                .emit_instr_reg_reg("    movq", src_name, target_name);
                        }
                    }
                    // If same register, nothing to do
                } else if let Some(slot) = self.state.get_slot(v.0) {
                    // Delegate to value_to_reg: it handles allocas (leaq with
                    // over-alignment), vector values, and — critically — loads
                    // small (4-byte) slots with `movl` instead of `movq`.
                    // Loading a 32-bit-stored value with `movq` reads 4 stale
                    // bytes past the store, producing garbage in the upper
                    // half that shows up in later 64-bit operations (xorq,
                    // addq, ...).
                    self.value_to_reg(v, target_name);
                } else if self.state.reg_cache.acc_has(v.0, false)
                    || self.state.reg_cache.acc_has(v.0, true)
                {
                    self.state
                        .out
                        .emit_instr_reg_reg("    movq", "rax", target_name);
                } else {
                    // Value not in any register, stack slot, or accumulator cache.
                    // Fall back to loading via the accumulator path instead of
                    // silently zeroing the register.
                    self.operand_to_rax(op);
                    self.state
                        .out
                        .emit_instr_reg_reg("    movq", "rax", target_name);
                }
            }
        }
    }

    /// Load an operand into a target register using 32-bit movl (for I32/U32).
    /// Uses movl instead of movq for both register-to-register and stack loads,
    /// reducing memory bandwidth and breaking false dependencies.
    pub(super) fn operand_to_reg_32(&mut self, op: &Operand, target: PhysReg) {
        let target_name = phys_reg_name(target);
        let target_32 = phys_reg_name_32(target);
        match op {
            Operand::Const(c) => match c {
                IrConst::I8(0)
                | IrConst::I16(0)
                | IrConst::I32(0)
                | IrConst::I64(0)
                | IrConst::Zero => self
                    .state
                    .emit_fmt(format_args!("    xorl %{0}, %{0}", target_32)),
                IrConst::I8(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, target_name)
                }
                IrConst::I16(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, target_name)
                }
                IrConst::I32(v) => {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movq", *v as i64, target_name)
                }
                _ => {
                    self.operand_to_callee_reg(op, target);
                    return;
                }
            },
            Operand::Value(v) => {
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if reg.0 != target.0 {
                        if is_xmm_reg(reg) {
                            let xmm_name = phys_reg_name(reg);
                            self.state
                                .emit_fmt(format_args!("    movq %{}, %{}", xmm_name, target_name));
                        } else {
                            // Use movq for register-to-register to preserve sign bits.
                            // movl zero-extends upper 32 bits, losing sign information
                            // for I32 values that later flow into 64-bit operations.
                            let src_name = phys_reg_name(reg);
                            self.state
                                .out
                                .emit_instr_reg_reg("    movq", src_name, target_name);
                        }
                    }
                } else if let Some(slot) = self.state.get_slot(v.0) {
                    if self.state.is_alloca(v.0) {
                        // Allocas are addresses, use 64-bit
                        self.operand_to_callee_reg(op, target);
                        return;
                    } else {
                        // Use movq to preserve sign bits in upper 32 bits.
                        // movl zero-extends, which corrupts negative I32 values
                        // that later flow into 64-bit operations.
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movq", slot.0, target_name);
                    }
                } else if self.state.reg_cache.acc_has(v.0, false)
                    || self.state.reg_cache.acc_has(v.0, true)
                {
                    self.state
                        .out
                        .emit_instr_reg_reg("    movq", "rax", target_name);
                } else {
                    self.state
                        .emit_fmt(format_args!("    xorl %{0}, %{0}", target_32));
                }
            }
        }
    }

    /// Load an operand into %rax. Uses the register cache to skip the load
    /// if the value is already in %rax.
    pub(super) fn operand_to_rax(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => {
                self.state.reg_cache.invalidate_acc();
                match c {
                    IrConst::I8(v) if *v == 0 => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I16(v) if *v == 0 => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I32(v) if *v == 0 => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I64(0) => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I8(v) => self.emit_imm_to_gpr(*v as i64, "rax", "eax"),
                    IrConst::I16(v) => self.emit_imm_to_gpr(*v as i64, "rax", "eax"),
                    IrConst::I32(v) => self.emit_imm_to_gpr(*v as i64, "rax", "eax"),
                    IrConst::I64(v) => self.emit_imm_to_gpr(*v, "rax", "eax"),
                    IrConst::F32(v) => {
                        let bits = v.to_bits() as u64;
                        if bits == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movq", bits as i64, "rax");
                        }
                    }
                    IrConst::F64(v) => {
                        let bits = v.to_bits();
                        if bits == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", bits as i64, "rax");
                        }
                    }
                    // LongDouble at computation level is treated as F64
                    IrConst::LongDouble(v, _) => {
                        let bits = v.to_bits();
                        if bits == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", bits as i64, "rax");
                        }
                    }
                    // Decimal FP bit containers materialize like 64-bit
                    // integer immediates (BID patterns are opaque bits).
                    IrConst::D32(v) => {
                        let bits = *v as u64;
                        if bits == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movl", *v as i64, "eax");
                        }
                    }
                    IrConst::D64(v) => {
                        if *v == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", *v as i64, "rax");
                        }
                    }
                    IrConst::I128(v) => {
                        // Truncate to low 64 bits for rax-only path
                        let low = *v as i64;
                        if low == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else if low >= i32::MIN as i64 && low <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", low, "rax");
                        } else {
                            self.state.out.emit_instr_imm_reg("    movabsq", low, "rax");
                        }
                    }
                    IrConst::Zero => self.state.emit("    xorl %eax, %eax"),
                }
            }
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                // Check cache: skip load if value is already in %rax
                if self.state.reg_cache.acc_has(v.0, is_alloca) {
                    return;
                }
                // Check secondary cache: if value is in %rcx, use movq %rcx, %rax
                // (3 bytes) instead of loading from stack (7-8 bytes)
                if self.state.reg_cache.sec_has(v.0, is_alloca) {
                    self.state.out.emit_instr_reg_reg("    movq", "rcx", "rax");
                    self.state.reg_cache.set_acc(v.0, is_alloca);
                    return;
                }
                // Check register allocation: load from assigned register
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(reg) {
                        let reg_name = phys_reg_name(reg);
                        self.state
                            .emit_fmt(format_args!("    movq %{}, %rax", reg_name));
                    } else {
                        let reg_name = phys_reg_name(reg);
                        self.state
                            .out
                            .emit_instr_reg_reg("    movq", reg_name, "rax");
                    }
                    self.state.reg_cache.set_acc(v.0, false);
                } else if self.state.get_slot(v.0).is_some() {
                    self.value_to_reg(v, "rax");
                    self.state.reg_cache.set_acc(v.0, is_alloca);
                } else {
                    // A rematerialisable GlobalAddr deliberately has no home:
                    // its address computation is omitted at the definition and
                    // "every consumer is expected to rebuild it" (the contract
                    // documented at value_to_reg_inner).  operand_to_rax is a
                    // consumer like any other — the call-argument staging
                    // highway reaches it — so rebuild the address here instead
                    // of failing a handoff that the def side is entitled to
                    // produce (fresh GlobalAddr definitions spliced in by IR
                    // passes, e.g. fortify_fold's stripped puts literals, may
                    // legitimately stay slot-less in variadic frames where the
                    // packed-slot tiers skip them).  value_to_reg still fails
                    // loudly if the value is genuinely unmaterialisable, so
                    // the session-26 gate's soundness guarantee is preserved.
                    let is_global_addr = self.get_defining_instruction(v.0).is_some_and(|inst| {
                        matches!(inst, crate::ir::reexports::Instruction::GlobalAddr { .. })
                    });
                    if is_global_addr {
                        self.value_to_reg(v, "rax");
                        self.state.reg_cache.set_acc(v.0, is_alloca);
                        return;
                    }
                    // Fallback for coalescing failures: if the value is defined
                    // by a same-size Cast (U64<->I64, etc) or a Copy, and its
                    // source HAS a home, materialize the source instead. This
                    // handles the kernel's print_cfs_group_stats where a U64
                    // Load is cast to I64 for a Cmp Slt — the cast had no home
                    // due to RA coalesce miss, but its source does.
                    // Clone def info to avoid borrow conflicts.
                    //
                    // Float-involving casts are NOT bit-identical: (long
                    // long)3.0 must cvttsd2si to 3, not re-publish the raw
                    // bits 0x4008000000000000; (double)ptr is cvtsi2sdq, not
                    // a movq bitcast (check_ptr_to_float_cast precedent).
                    // cast_is_bitidentical_nop is the shared predicate every
                    // other same-size-cast fold in this repo already honors;
                    // anything it rejects still reaches the loud HARD GATE
                    // below rather than silently fabricating wrong values.
                    let fallback_src = {
                        if let Some(def_inst) = self.get_defining_instruction(v.0) {
                            match def_inst {
                                crate::ir::reexports::Instruction::Cast {
                                    src,
                                    from_ty,
                                    to_ty,
                                    ..
                                } => {
                                    if IrType::cast_is_bitidentical_nop(*from_ty, *to_ty) {
                                        Some(src.clone())
                                    } else {
                                        None
                                    }
                                }
                                crate::ir::reexports::Instruction::Copy { src, .. } => {
                                    Some(src.clone())
                                }
                                _ => None,
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(src_op) = fallback_src {
                        // Check if src has a home
                        let src_has_home = match &src_op {
                            crate::ir::reexports::Operand::Value(src_v) => {
                                self.reg_assignments.contains_key(&src_v.0)
                                    || self.state.get_slot(src_v.0).is_some()
                                    || self
                                        .state
                                        .reg_cache
                                        .acc_has(src_v.0, self.state.is_alloca(src_v.0))
                                    || self
                                        .state
                                        .reg_cache
                                        .sec_has(src_v.0, self.state.is_alloca(src_v.0))
                            }
                            crate::ir::reexports::Operand::Const(_) => true,
                        };
                        if src_has_home {
                            self.operand_to_rax(&src_op);
                            self.state.reg_cache.set_acc(v.0, is_alloca);
                            return;
                        }
                    }
                    // HARD GATE (session 26): a live operand value with no
                    // register home, no stack slot and no accumulator-cache
                    // entry cannot be materialised — this spot used to emit a
                    // silent `xorl %eax,%eax`, fabricating zero and corrupting
                    // whatever consumed it (sqlite -O2 sqlite3KeyInfoAlloc:
                    // p->aSortFlags stored as NULL → SIGSEGV in
                    // sqlite3KeyInfoFromExprList).  Reaching here means a
                    // producer/consumer handoff is broken; fail loudly.
                    //
                    // Include the tail of the asm emitted so far: the
                    // instruction context around the failure is exactly what a
                    // soundness hunt needs, and it is otherwise lost because
                    // the panic aborts before the .s file is written.
                    let tail: Vec<&str> = {
                        let lines: Vec<&str> = self.state.out.buf.lines().collect();
                        let n = lines.len();
                        lines[n.saturating_sub(30)..].to_vec()
                    };
                    panic!(
                        "x86 codegen: operand_to_rax: value {} in function '{}' \
                         has no register home, no stack slot and no acc-cache \
                         entry — refusing to fabricate a value\n\
                         --- last {} asm lines ---\n{}",
                        v.0,
                        self.state.current_func_name,
                        tail.len(),
                        tail.join("\n")
                    );
                }
            }
        }
    }

    /// Load an integer operand into zero-extended %eax using 32-bit moves.
    /// Used by small integer returns where %rax's upper half must be zero but
    /// the value itself is only 32 bits.  Register and stack homes are loaded
    /// with `movl`, which is shorter than `movq` and avoids a 64-bit dependency.
    pub(super) fn operand_to_rax_i32(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => {
                self.state.reg_cache.invalidate_acc();
                if c.to_i64() == Some(0) {
                    self.state.emit("    xorl %eax, %eax");
                } else if let Some(v) = c.to_i64() {
                    self.state
                        .out
                        .emit_instr_imm_reg("    movl", v as i32 as i64, "eax");
                } else {
                    self.operand_to_rax(op);
                }
            }
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                if self.state.reg_cache.acc_has(v.0, is_alloca) {
                    return;
                }
                if self.state.reg_cache.sec_has(v.0, is_alloca) {
                    self.state.emit("    movl %ecx, %eax");
                    self.state.reg_cache.set_acc(v.0, is_alloca);
                    return;
                }
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    let name = phys_reg_name(reg);
                    if is_xmm_reg(reg) {
                        self.state.emit_fmt(format_args!("    movq %{name}, %rax"));
                    } else if self.value_types.get(&v.0).is_some_and(|t| t.size() <= 4) {
                        let name32 = phys_reg_name_32(reg);
                        // Identity `movl %eax, %eax` is a 2-byte no-op (and a
                        // false dependency). Skip it when the value is already
                        // in the return register.
                        if name32 != "eax" {
                            self.state
                                .emit_fmt(format_args!("    movl %{name32}, %eax"));
                        }
                    } else {
                        self.state.emit_fmt(format_args!("    movq %{name}, %rax"));
                    }
                } else if let Some(slot) = self.state.get_slot(v.0) {
                    // value_to_reg is a 64-bit loader and historically calls
                    // emitted `movq slot, %eax` when passed a 32-bit register.
                    // For a small integer return use a width-consistent load.
                    match self.value_types.get(&v.0).copied().unwrap_or(IrType::I64) {
                        IrType::I8 => {
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movsbq", slot.0, "eax")
                        }
                        IrType::U8 => {
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movzbl", slot.0, "eax")
                        }
                        IrType::I16 => {
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movswq", slot.0, "eax")
                        }
                        IrType::U16 => {
                            self.state
                                .out
                                .emit_instr_rbp_reg("    movzwl", slot.0, "eax")
                        }
                        IrType::I32 if self.needs_sext_values.contains(&v.0) => self
                            .state
                            .out
                            .emit_instr_rbp_reg("    movslq", slot.0, "rax"),
                        _ => self.state.out.emit_instr_rbp_reg("    movl", slot.0, "eax"),
                    }
                } else {
                    self.operand_to_rax(op);
                    return;
                }
                self.state.reg_cache.set_acc(v.0, false);
            }
        }
    }

    /// Store %rax to a value's location (register or stack slot).
    /// Register-only strategy: if the value has a register assignment (callee-saved or caller-saved),
    /// store ONLY to the register (skip the stack write). This eliminates redundant
    /// memory stores for register-allocated values. Values without a register
    /// assignment are stored to their stack slot as before.
    pub(super) fn store_rax_to(&mut self, dest: &Value) {
        if self
            .state
            .value_use_counts
            .get(dest.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0
        {
            self.state.reg_cache.set_acc(dest.0, false);
            return;
        }
        // Use movl (4 bytes) for small-slot values instead of movq (8 bytes).
        // This saves 1 byte per instruction (no REX prefix) and halves memory bandwidth.
        // Only safe when all loads of this value also use 32-bit form.
        let use_small = self.state.is_small_slot(dest.0);

        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(reg) {
                let reg_name = phys_reg_name(reg);
                self.state
                    .emit_fmt(format_args!("    movq %rax, %{}", reg_name));
            } else {
                // Always use movq for register stores, even for I32/U32 values.
                // movl zero-extends the upper 32 bits, which corrupts negative I32
                // values that later flow into 64-bit operations (pointer arithmetic,
                // imul) through callee-saved registers.
                let reg_name = phys_reg_name(reg);
                self.state
                    .out
                    .emit_instr_reg_reg("    movq", "rax", reg_name);
            }
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            if use_small {
                self.state.out.emit_instr_reg_rbp("    movl", "eax", slot.0);
            } else {
                self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
            }
        } else if self.state.is_accumulator_location(dest.0) {
            // Value is consumed by the very next instruction — skip the store.
            // The accumulator cache (set below) ensures the consumer finds it in %rax.
        } else {
            panic!(
                "x86 codegen: live value {} has no assigned location",
                dest.0
            );
        }
        self.state.reg_cache.set_acc(dest.0, false);
    }

    /// Store an XMM register holding a scalar F64/F32 value to a value's
    /// location, keeping the value in the SSE domain: `movsd`/`movss` for
    /// XMM-allocated destinations and stack slots, `movq`/`movd` for
    /// GPR-allocated destinations. Falls back to a move through %rax only
    /// when the destination has neither a register nor a slot (e.g.
    /// immediately-consumed values that must live in the accumulator).
    ///
    /// The direct paths do not touch %rax, so the accumulator cache is left
    /// untouched (matching the register-direct load/store paths); only the
    /// %rax fallback invalidates it (via store_rax_to).
    pub(super) fn store_xmm_to(&mut self, dest: &Value, src_xmm: &str, ty: IrType) {
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            let reg_name = phys_reg_name(reg);
            if is_xmm_reg(reg) {
                if reg_name != src_xmm {
                    // movapd (not movsd/movss): the reg-reg scalar forms
                    // preserve the destination's upper bits, creating a false
                    // dependency on its old value. movapd copies all 128 bits
                    // (dead above bit 63 for scalars) with no false dependency.
                    self.state
                        .emit_fmt(format_args!("    movapd %{}, %{}", src_xmm, reg_name));
                }
            } else if ty == IrType::F32 || ty == IrType::D32 {
                self.state.emit_fmt(format_args!(
                    "    movd %{}, %{}",
                    src_xmm,
                    phys_reg_name_32(reg)
                ));
            } else {
                self.state
                    .emit_fmt(format_args!("    movq %{}, %{}", src_xmm, reg_name));
            }
            return;
        }
        if !self.state.is_alloca(dest.0) && !self.state.vector_values.contains(&dest.0) {
            if let Some(slot) = self.state.get_slot(dest.0) {
                let instr = if matches!(ty, IrType::F32 | IrType::D32) {
                    "    movss"
                } else {
                    "    movsd"
                };
                self.state.out.emit_instr_reg_rbp(instr, src_xmm, slot.0);
                return;
            }
        }
        // Fallback: route through %rax like store_rax_to does.
        if ty == IrType::F32 || ty == IrType::D32 {
            self.state
                .emit_fmt(format_args!("    movd %{}, %eax", src_xmm));
        } else {
            self.state
                .emit_fmt(format_args!("    movq %{}, %rax", src_xmm));
        }
        self.state.reg_cache.invalidate_acc();
        self.store_rax_to(dest);
    }

    /// Store %eax (32-bit) to a value's location using movl instead of movq.
    /// Breaks false dependencies on upper 32 bits and halves memory bandwidth
    /// for stack spills. Only safe when all subsequent loads of this value also
    /// use 32-bit form (movl or movslq).
    pub(super) fn store_eax_to(&mut self, dest: &Value) {
        if self
            .state
            .value_use_counts
            .get(dest.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0
        {
            self.state.reg_cache.set_acc(dest.0, false);
            return;
        }
        if let Some(&reg) = self.reg_assignments.get(&dest.0) {
            if is_xmm_reg(reg) {
                let reg_name = phys_reg_name(reg);
                self.state
                    .emit_fmt(format_args!("    movq %rax, %{}", reg_name));
            } else {
                // GPR register: use movq to preserve all 64 bits of the accumulator.
                // movl would zero-extend the upper 32 bits, which loses sign
                // information for negative I32 values that later flow into 64-bit
                // operations (pointer arithmetic, imul) through callee-saved regs.
                let reg_name = phys_reg_name(reg);
                self.state
                    .out
                    .emit_instr_reg_reg("    movq", "rax", reg_name);
            }
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            if self.state.is_small_slot(dest.0) {
                // 4-byte slot: writing all 8 bytes of %rax would clobber the
                // neighbouring slot. The low 4 bytes carry the entire value;
                // sign-extending consumers reload with movslq (needs-sext).
                self.state.out.emit_instr_reg_rbp("    movl", "eax", slot.0);
            } else {
                // No register: store 64 bits to preserve sign extension.
                self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
            }
        } else if !self.state.is_accumulator_location(dest.0) {
            panic!(
                "x86 codegen: live value {} has no assigned location",
                dest.0
            );
        }
        self.state.reg_cache.set_acc(dest.0, false);
    }

    /// Load a value to %eax using movl (32-bit) from stack, or movl %regd, %eax
    /// from a register. Pairs with store_eax_to for consistent 32-bit spill/reload.
    pub(super) fn operand_to_eax(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => {
                self.state.reg_cache.invalidate_acc();
                match c {
                    IrConst::I8(v) if *v == 0 => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I16(v) if *v == 0 => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I32(v) if *v == 0 => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I64(0) => self.state.emit("    xorl %eax, %eax"),
                    IrConst::I8(v) => self
                        .state
                        .out
                        .emit_instr_imm_reg("    movl", *v as i64, "eax"),
                    IrConst::I16(v) => self
                        .state
                        .out
                        .emit_instr_imm_reg("    movl", *v as i64, "eax"),
                    IrConst::I32(v) => self
                        .state
                        .out
                        .emit_instr_imm_reg("    movl", *v as i64, "eax"),
                    IrConst::Zero => self.state.emit("    xorl %eax, %eax"),
                    _ => {
                        // Fallback to 64-bit for other const types
                        self.operand_to_rax(op);
                        return;
                    }
                }
            }
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                if self.state.reg_cache.acc_has(v.0, is_alloca) {
                    return; // cache hit
                }
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    if is_xmm_reg(reg) {
                        let reg_name = phys_reg_name(reg);
                        self.state
                            .emit_fmt(format_args!("    movq %{}, %rax", reg_name));
                    } else {
                        // GPR: movl %regd, %eax (32-bit)
                        let reg_name_32 = phys_reg_name_32(reg);
                        self.state
                            .emit_fmt(format_args!("    movl %{}, %eax", reg_name_32));
                    }
                    self.state.reg_cache.set_acc(v.0, false);
                } else if self.state.get_slot(v.0).is_some() {
                    // Load from stack using movl (32-bit)
                    let slot = self.state.get_slot(v.0).unwrap();
                    self.state.out.emit_instr_rbp_reg("    movl", slot.0, "eax");
                    self.state.reg_cache.set_acc(v.0, is_alloca);
                } else {
                    self.state.emit("    xorl %eax, %eax");
                    self.state.reg_cache.invalidate_acc();
                }
            }
        }
    }

    /// Check if a scale value is valid for x86-64 SIB (Scale-Index-Base) encoding.
    /// Valid scales are 1, 2, 4, or 8 (encoded in 2 bits).
    pub(super) fn is_valid_sib_scale(scale: i64) -> bool {
        matches!(scale, 1 | 2 | 4 | 8)
    }

    /// Helper function to get the instruction that defines a value.
    /// Returns None if the value isn't defined by an instruction in the current function.
    pub(super) fn get_defining_instruction(
        &self,
        val_id: u32,
    ) -> Option<&crate::ir::reexports::Instruction> {
        let func_ptr = self.current_func?;
        let func = unsafe { &*func_ptr };

        for block in &func.blocks {
            for inst in &block.instructions {
                use crate::ir::reexports::Instruction;
                let dest_id = match inst {
                    Instruction::BinOp { dest, .. } => Some(dest.0),
                    Instruction::UnaryOp { dest, .. } => Some(dest.0),
                    Instruction::Cast { dest, .. } => Some(dest.0),
                    Instruction::GetElementPtr { dest, .. } => Some(dest.0),
                    Instruction::Load { dest, .. } => Some(dest.0),
                    Instruction::Cmp { dest, .. } => Some(dest.0),
                    Instruction::Phi { dest, .. } => Some(dest.0),
                    // GlobalAddr dests must be found: rematerialisable
                    // addresses have deliberately no home, so every
                    // consumer-side rebuild (value_to_reg / operand_to_rax)
                    // depends on this lookup locating the definition.  The
                    // missing arm made the documented rebuild contract dead
                    // code and forced slot assignment for every spliced
                    // GlobalAddr (fortify_fold's stripped puts literals in
                    // variadic frames hit exactly that wall).
                    Instruction::GlobalAddr { dest, .. } => Some(dest.0),
                    _ => None,
                };

                if dest_id == Some(val_id) {
                    return Some(inst);
                }
            }
        }
        None
    }

    /// Analyze function to detect IVSR pointer patterns.
    /// Called once before codegen starts.
    ///
    /// NOTE: This analysis currently does not find any patterns because phi nodes
    /// are eliminated before codegen runs. For Phase 9b to work properly, this
    /// detection would need to run as an IR transformation pass BEFORE phi elimination.
    /// See docs/phase9b_implementation_notes.md for details.
    pub(super) fn analyze_ivsr_pointers(&mut self, _func: &IrFunction) {
        // Disabled: Phi nodes are eliminated before codegen, so this detection
        // cannot find IVSR patterns. Left in place as infrastructure for future
        // implementation via IR transformation pass.
    }

    /// Detect if a phi node matches the IVSR pointer pattern:
    /// %ptr = Phi(%init_ptr from preheader, %ptr_next from backedge)
    /// where %ptr_next = GEP(%ptr, const_stride)
    fn detect_ivsr_pattern(
        &self,
        dest: &Value,
        incoming: &[(Operand, BlockId)],
        func: &IrFunction,
        header_block: BlockId,
    ) -> Option<IvsrPointerInfo> {
        use crate::ir::reexports::Instruction;

        // Extract the two incoming edges
        let (init_op, _init_block) = &incoming[0];
        let (backedge_op, backedge_block) = &incoming[1];

        // Backedge value should be defined by a GEP
        let backedge_val = match backedge_op {
            Operand::Value(v) => v,
            _ => return None,
        };

        // Find the GEP instruction that defines the backedge value
        let (gep_base, stride) = self.find_gep_for_ivsr(backedge_val.0, func)?;

        // Verify GEP base is the phi itself (recursive pattern)
        if gep_base.0 != dest.0 {
            return None;
        }

        // Extract base pointer from init value
        let (base_ptr, init_offset) = self.extract_base_from_init(init_op, func)?;

        Some(IvsrPointerInfo {
            phi_val: *dest,
            base_ptr,
            stride,
            init_offset,
            header_block,
            backedge_block: *backedge_block,
        })
    }

    /// Find a GEP instruction that defines a value and extract its base and offset.
    /// Returns (base, stride) where stride is a constant offset.
    fn find_gep_for_ivsr(&self, val_id: u32, func: &IrFunction) -> Option<(Value, i64)> {
        use crate::ir::reexports::Instruction;

        for block in &func.blocks {
            for inst in &block.instructions {
                if let Instruction::GetElementPtr {
                    dest, base, offset, ..
                } = inst
                {
                    if dest.0 == val_id {
                        // Extract constant offset
                        let stride = match offset {
                            Operand::Const(c) => c.to_i64()?,
                            _ => return None,
                        };
                        return Some((*base, stride));
                    }
                }
            }
        }
        None
    }

    /// Extract the base pointer from the init value of an IVSR phi.
    /// Handles both direct base pointers and GEP-computed init values.
    fn extract_base_from_init(&self, init_op: &Operand, func: &IrFunction) -> Option<(Value, i64)> {
        use crate::ir::reexports::Instruction;

        match init_op {
            Operand::Value(v) => {
                // Check if init value is a GEP (handles cases like %init = GEP(%arr, 0))
                for block in &func.blocks {
                    for inst in &block.instructions {
                        if let Instruction::GetElementPtr {
                            dest, base, offset, ..
                        } = inst
                        {
                            if dest.0 == v.0 {
                                let init_offset = match offset {
                                    Operand::Const(c) => c.to_i64().unwrap_or(0),
                                    _ => 0,
                                };
                                return Some((*base, init_offset));
                            }
                        }
                    }
                }
                // Not a GEP - just a direct pointer value
                Some((*v, 0))
            }
            _ => None,
        }
    }

    /// Associate IVSR pointer phis with their corresponding loop counter IVs.
    fn associate_pointers_with_counters(&mut self, func: &IrFunction) {
        let ptr_phis: Vec<(u32, BlockId)> = self
            .ivsr_pointers
            .iter()
            .map(|(&id, info)| (id, info.header_block))
            .collect();

        for (ptr_phi_id, header_block) in ptr_phis {
            // Find integer IV phi in same header block
            if let Some(counter_iv) = self.find_counter_in_header(header_block, func) {
                self.pointer_to_counter.insert(ptr_phi_id, counter_iv.0);
            }
        }
    }

    /// Find an integer induction variable phi in the given header block.
    fn find_counter_in_header(&self, header: BlockId, func: &IrFunction) -> Option<Value> {
        use crate::ir::reexports::Instruction;

        let header_block = &func.blocks[header.0 as usize];

        for inst in &header_block.instructions {
            if let Instruction::Phi { dest, ty, incoming } = inst {
                // Look for integer phis (I32, I64, U32, U64)
                if !matches!(ty, IrType::I32 | IrType::I64 | IrType::U32 | IrType::U64) {
                    continue;
                }

                // Check if it's incremented in the backedge (simple heuristic)
                if self.is_induction_variable(dest, incoming, func) {
                    return Some(*dest);
                }
            }
        }

        None
    }

    /// Check if a phi node is an induction variable (incremented by a constant).
    fn is_induction_variable(
        &self,
        dest: &Value,
        incoming: &[(Operand, BlockId)],
        func: &IrFunction,
    ) -> bool {
        use crate::ir::reexports::Instruction;

        if incoming.len() != 2 {
            return false;
        }

        // Get the backedge value (typically the second incoming edge)
        let backedge_val = match &incoming[1].0 {
            Operand::Value(v) => v,
            _ => return false,
        };

        // Check if backedge value is defined by Add/Sub with the phi itself
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    Instruction::BinOp {
                        dest: bd,
                        op,
                        lhs,
                        rhs,
                        ..
                    } if bd.0 == backedge_val.0 => {
                        // Check for %i_next = Add(%i, const) or Add(const, %i)
                        let is_add_or_sub = matches!(op, IrBinOp::Add | IrBinOp::Sub);
                        if !is_add_or_sub {
                            return false;
                        }

                        let has_phi = match (lhs, rhs) {
                            (Operand::Value(v), Operand::Const(_)) => v.0 == dest.0,
                            (Operand::Const(_), Operand::Value(v)) => v.0 == dest.0,
                            _ => false,
                        };

                        return has_phi;
                    }
                    _ => continue,
                }
            }
        }

        false
    }

    /// Load an operand directly into %rcx, avoiding the push/pop pattern.
    /// This is the key optimization: instead of loading to rax, pushing, loading
    /// the other operand to rax, moving rax->rcx, then popping rax, we load
    /// directly to rcx with a single instruction.
    pub(super) fn operand_to_rcx(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => {
                self.state.reg_cache.invalidate_sec();
                match c {
                    IrConst::I8(v) if *v == 0 => self.state.emit("    xorl %ecx, %ecx"),
                    IrConst::I16(v) if *v == 0 => self.state.emit("    xorl %ecx, %ecx"),
                    IrConst::I32(v) if *v == 0 => self.state.emit("    xorl %ecx, %ecx"),
                    IrConst::I64(0) => self.state.emit("    xorl %ecx, %ecx"),
                    IrConst::I8(v) => self.emit_imm_to_gpr(*v as i64, "rcx", "ecx"),
                    IrConst::I16(v) => self.emit_imm_to_gpr(*v as i64, "rcx", "ecx"),
                    IrConst::I32(v) => self.emit_imm_to_gpr(*v as i64, "rcx", "ecx"),
                    IrConst::I64(v) => self.emit_imm_to_gpr(*v, "rcx", "ecx"),
                    IrConst::F32(v) => {
                        let bits = v.to_bits() as u64;
                        if bits == 0 {
                            self.state.emit("    xorl %ecx, %ecx");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movq", bits as i64, "rcx");
                        }
                    }
                    IrConst::F64(v) => {
                        let bits = v.to_bits();
                        if bits == 0 {
                            self.state.emit("    xorl %ecx, %ecx");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", bits as i64, "rcx");
                        }
                    }
                    IrConst::LongDouble(v, _) => {
                        let bits = v.to_bits();
                        if bits == 0 {
                            self.state.emit("    xorl %ecx, %ecx");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", bits as i64, "rcx");
                        }
                    }
                    IrConst::I128(v) => {
                        let low = *v as i64;
                        if low == 0 {
                            self.state.emit("    xorl %ecx, %ecx");
                        } else if low >= i32::MIN as i64 && low <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", low, "rcx");
                        } else {
                            self.state.out.emit_instr_imm_reg("    movabsq", low, "rcx");
                        }
                    }
                    // Decimal FP bit containers materialize like integer
                    // immediates (BID patterns are opaque bits).
                    IrConst::D32(v) => {
                        if *v == 0 {
                            self.state.emit("    xorl %ecx, %ecx");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movl", *v as i64, "ecx");
                        }
                    }
                    IrConst::D64(v) => {
                        if *v == 0 {
                            self.state.emit("    xorl %ecx, %ecx");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", *v as i64, "rcx");
                        }
                    }
                    IrConst::Zero => self.state.emit("    xorl %ecx, %ecx"),
                }
            }
            Operand::Value(v) => {
                let is_alloca = self.state.is_alloca(v.0);
                // Check if already in %rcx (sec cache hit)
                if self.state.reg_cache.sec_has(v.0, is_alloca) {
                    // Already in %rcx — nothing to do
                    return;
                }
                // Check register allocation: load from callee-saved register
                if let Some(&reg) = self.reg_assignments.get(&v.0) {
                    let reg_name = phys_reg_name(reg);
                    self.state
                        .out
                        .emit_instr_reg_reg("    movq", reg_name, "rcx");
                } else if self.state.get_slot(v.0).is_some() {
                    self.value_to_reg(v, "rcx");
                } else if self.state.reg_cache.acc_has(v.0, false)
                    || self.state.reg_cache.acc_has(v.0, true)
                {
                    self.state.out.emit_instr_reg_reg("    movq", "rax", "rcx");
                } else {
                    // value_to_reg rebuilds a rematerialisable GlobalAddr (a
                    // value with deliberately no home) and otherwise fails
                    // loudly. The old `xorl %ecx,%ecx` silently fabricated 0
                    // for `&buf[k]` and corrupted the address base — the same
                    // disease class as the operand_to_rax hard gate
                    // (torture execute/20000412-6.c: `buf+3` decoded as `6`).
                    // Credit: Agent C torture triage T01-family.
                    self.value_to_reg(v, "rcx");
                }
                // Record what's now in %rcx
                self.state.reg_cache.set_sec(v.0, is_alloca);
            }
        }
    }

    /// PF-16: materialise a GPR immediate. Non-negative values in `[0, i32::MAX]`
    /// use `movl` (zero-extends, two bytes shorter than `movq $imm32`); negative
    /// values keep `movq` (sign-extend); values outside imm32 use `movabsq`.
    pub(super) fn emit_imm_to_gpr(&mut self, val: i64, name64: &str, name32: &str) {
        if val == 0 {
            self.state
                .emit_fmt(format_args!("    xorl %{0}, %{0}", name32));
        } else if val >= 0 && val <= i32::MAX as i64 {
            self.state
                .emit_fmt(format_args!("    movl ${}, %{}", val, name32));
        } else if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
            self.state.out.emit_instr_imm_reg("    movq", val, name64);
        } else {
            self.state
                .out
                .emit_instr_imm_reg("    movabsq", val, name64);
        }
    }

    /// Check if an operand is a small constant that fits in a 32-bit immediate.
    /// Returns the immediate value if it fits, None otherwise.
    pub(super) fn const_as_imm32(op: &Operand) -> Option<i64> {
        Self::const_as_imm32_typed(op, false)
    }

    /// Immediate-32 encoding of a constant operand.
    ///
    /// 64-bit instructions sign-extend imm32, so only values in the signed
    /// i32 range are representable there. 32-bit instructions use the imm32
    /// bit pattern verbatim, so any value in [0, u32::MAX] is encodable
    /// (unsigned constants live in I64: `mask == 0xFFFFFFFF` now emits
    /// `cmpl $4294967295, %eax` instead of movabs+cmp).
    pub(super) fn const_as_imm32_typed(op: &Operand, is_32bit_op: bool) -> Option<i64> {
        match op {
            Operand::Const(c) => {
                let val = match c {
                    IrConst::I8(v) => *v as i64,
                    IrConst::I16(v) => *v as i64,
                    IrConst::I32(v) => *v as i64,
                    IrConst::I64(v) => *v,
                    IrConst::Zero => 0,
                    _ => return None,
                };
                if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
                    return Some(val);
                }
                if is_32bit_op && val >= 0 && val <= u32::MAX as i64 {
                    // Exact 32-bit bit pattern for a 32-bit operation.
                    return Some(val);
                }
                None
            }
            _ => None,
        }
    }

    /// Load a Value into a named register. For allocas, loads the address (leaq);
    /// for register-allocated values, copies from the callee-saved register;
    /// for regular values, loads the full 8 bytes from the stack slot with movq.
    ///
    /// The prologue ensures that sub-64-bit parameters are sign/zero-extended
    /// to 64 bits before being stored, so movq always reads valid data.
    pub(super) fn value_to_reg(&mut self, val: &Value, reg: &str) {
        self.value_to_reg_inner(val, reg, 0);
    }

    fn value_to_reg_inner(&mut self, val: &Value, reg: &str, depth: u8) {
        if depth >= 32 {
            panic!(
                "x86 codegen: cyclic or excessive Copy chain for value {}",
                val.0
            );
        }
        // Every materialisation below is a 64-bit `movq`/`leaq` into `reg`, so
        // `reg` must name a 64-bit register. Passing a 32-bit name produced
        // `movq %r8, %eax`, which is not encodable; the integrated assembler
        // rejected it and zlib-ng's slide_hash_avx2.c failed to build. Catch
        // the mistake at its source instead of in the assembler.
        debug_assert!(
            reg.starts_with('r') && !reg.ends_with('d'),
            "value_to_reg expects a 64-bit register name, got %{reg}"
        );
        // Check register allocation first (allocas are never register-allocated)
        if let Some(&phys_reg) = self.reg_assignments.get(&val.0) {
            if is_xmm_reg(phys_reg) {
                // XMM → GPR: movq %xmmN, %reg
                let xmm_name = phys_reg_name(phys_reg);
                self.state
                    .emit_fmt(format_args!("    movq %{}, %{}", xmm_name, reg));
            } else {
                let reg_name = phys_reg_name(phys_reg);
                if reg_name != reg {
                    self.state.out.emit_instr_reg_reg("    movq", reg_name, reg);
                }
            }
            return;
        }
        if let Some(slot) = self.state.get_slot(val.0) {
            if self.state.is_alloca(val.0) {
                if let Some(align) = self.state.alloca_over_align(val.0) {
                    // Over-aligned alloca: compute aligned address within the
                    // oversized stack slot. The slot has (align - 1) extra bytes
                    // to guarantee we can find an aligned address within it.
                    self.state.out.emit_instr_rbp_reg("    leaq", slot.0, reg);
                    self.state
                        .out
                        .emit_instr_imm_reg("    addq", (align - 1) as i64, reg);
                    self.state
                        .out
                        .emit_instr_imm_reg("    andq", -(align as i64), reg);
                } else {
                    self.state.out.emit_instr_rbp_reg("    leaq", slot.0, reg);
                }
            } else if self.state.vector_values.contains(&val.0) {
                // Vector value: use leaq to get address (vectors are stored directly at slot)
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, reg);
            } else if matches!(
                self.value_types.get(&val.0),
                Some(IrType::I8)
                    | Some(IrType::U8)
                    | Some(IrType::I16)
                    | Some(IrType::U16)
                    | Some(IrType::I32)
                    | Some(IrType::U32)
                    | Some(IrType::F32)
            ) {
                // Small-typed value: load it WIDTH-CONSISTENTLY with the
                // type-aware stores that wrote it. The slot is always 8
                // bytes (stores write movq OR the type-aware movb/movw/movl),
                // so reading the low N bytes with the matching extension is
                // correct in both cases. A plain `movq` load of a value that
                // was stored with movb/movw/movl would pull stale bytes out
                // of the slot's upper half and corrupt any later 64-bit
                // consumer.
                //   * I8:  movsbq (signed) / movzbl (unsigned)
                //   * I16: movswq (signed) / movzwl (unsigned)
                //   * I32: movslq when the value is consumed in a 64-bit
                //          context (needs-sext analysis), else movl
                //   * U32/F32: movl (zero-extends implicitly)
                let ty = self.value_types.get(&val.0).copied().unwrap_or(IrType::I64);
                let needs_sext = ty == IrType::I32
                    && !self.needs_sext_values.is_empty()
                    && self.needs_sext_values.contains(&val.0);
                match ty {
                    IrType::I8 => self.state.out.emit_instr_rbp_reg("    movsbq", slot.0, reg),
                    IrType::U8 => {
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzbl", slot.0, reg_name_to_32(reg))
                    }
                    IrType::I16 => self.state.out.emit_instr_rbp_reg("    movswq", slot.0, reg),
                    IrType::U16 => {
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movzwl", slot.0, reg_name_to_32(reg))
                    }
                    _ if needs_sext => self.state.out.emit_instr_rbp_reg("    movslq", slot.0, reg),
                    _ => {
                        let reg32 = reg_name_to_32(reg);
                        self.state.out.emit_instr_rbp_reg("    movl", slot.0, reg32);
                    }
                }
            } else if self.state.f128_direct_slots.contains(&val.0)
                || self.value_types.get(&val.0) == Some(&IrType::F128)
            {
                // F128 (long double) value: its 16-byte slot holds the
                // canonical 80-bit x87 image, whose low quad is the MANTISSA,
                // not an IEEE double bit pattern. The GP-register view of an
                // F128 value is its truncated f64 copy (the convention every
                // producer establishes with fstpl into %rax), so materialise
                // it by a REAL conversion — fldt + fstpl — instead of a
                // movq bit-copy of the mantissa. The bit-copy made
                // 41.0L's low quad 0xa400000000000000 read back as a
                // negative denormal double: `v5 != ld` misfired and the
                // (double) print of an intact 41.0L showed -0.0
                // (va-arg-pack-1, both levels >= O1).
                self.state.out.emit_instr_rbp("    fldt", slot.0);
                self.state.emit("    subq $8, %rsp");
                self.state.emit("    fstpl (%rsp)");
                if reg == "rax" {
                    self.state.emit("    popq %rax");
                } else {
                    self.state
                        .emit_fmt(format_args!("    movq (%rsp), %{}", reg));
                    self.state.emit("    addq $8, %rsp");
                }
                self.state.reg_cache.set_acc(val.0, false);
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, reg);
            }
        } else {
            // Try to resolve through Copy or same-size Cast (U64<->I64 no-op).
            // Float-involving same-size casts are value conversions, never
            // bitcasts — cast_is_bitidentical_nop is the shared predicate
            // (see operand_to_rax's fallback for the full rationale).
            let source = self
                .get_defining_instruction(val.0)
                .and_then(|inst| match inst {
                    crate::ir::reexports::Instruction::Copy { src, .. } => Some(src.clone()),
                    crate::ir::reexports::Instruction::Cast {
                        src,
                        from_ty,
                        to_ty,
                        ..
                    } if IrType::cast_is_bitidentical_nop(*from_ty, *to_ty) => Some(src.clone()),
                    _ => None,
                });
            match source {
                Some(crate::ir::reexports::Operand::Value(src)) => {
                    self.value_to_reg_inner(&src, reg, depth + 1);
                }
                Some(constant @ crate::ir::reexports::Operand::Const(_)) => {
                    self.operand_to_reg(&constant, reg);
                }
                None => {
                    // A rematerialisable GlobalAddr deliberately has no home:
                    // its address computation was omitted at the definition
                    // and every consumer is expected to rebuild it. The
                    // audited-Add interception in generation.rs is NOT the
                    // only consumer shape — emit_gep bases, fused int-madd
                    // operands and operand_to_rcx all reach this generic
                    // loader (torture execute/20000412-6.c). Rebuild the
                    // address here; anything else still fails loudly.
                    // Credit: Agent C torture triage.
                    let gaddr_name = self.get_defining_instruction(val.0).and_then(|inst| {
                        if let crate::ir::reexports::Instruction::GlobalAddr { name, .. } = inst {
                            Some(name.clone())
                        } else {
                            None
                        }
                    });
                    if let Some(name) = gaddr_name {
                        self.emit_global_addr_into_reg(&name, reg);
                        return;
                    }
                    panic!(
                        "x86 codegen: value {} has no register, stack slot, Copy, or \
                         GlobalAddr definition",
                        val.0
                    );
                }
            }
        }
    }

    // --- 128-bit integer helpers ---
    // Convention: 128-bit values use %rax (low 64 bits) and %rdx (high 64 bits).
    // Stack slots for 128-bit values are 16 bytes: slot(%rbp) = low, slot+8(%rbp) = high.

    /// Load a 128-bit operand into %rax (low) and %rdx (high).
    pub(super) fn operand_to_rax_rdx(&mut self, op: &Operand) {
        match op {
            Operand::Const(c) => {
                match c {
                    IrConst::I128(v) => {
                        let low = *v as u64 as i64;
                        let high = (*v >> 64) as u64 as i64;
                        if low == 0 {
                            self.state.emit("    xorl %eax, %eax");
                        } else if low >= i32::MIN as i64 && low <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", low, "rax");
                        } else {
                            self.state.out.emit_instr_imm_reg("    movabsq", low, "rax");
                        }
                        if high == 0 {
                            self.state.emit("    xorl %edx, %edx");
                        } else if high >= i32::MIN as i64 && high <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", high, "rdx");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", high, "rdx");
                        }
                    }
                    IrConst::Zero => {
                        self.state.emit("    xorl %eax, %eax");
                        self.state.emit("    xorl %edx, %edx");
                    }
                    _ => {
                        // Smaller constant: load into rax, zero/sign-extend to rdx
                        self.operand_to_rax(op);
                        self.state.emit("    xorl %edx, %edx");
                    }
                }
            }
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    if self.state.is_alloca(v.0) {
                        // Alloca: load the address (not a 128-bit value itself)
                        if let Some(align) = self.state.alloca_over_align(v.0) {
                            // Over-aligned alloca: compute aligned address
                            self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rax");
                            self.state.out.emit_instr_imm_reg(
                                "    addq",
                                (align - 1) as i64,
                                "rax",
                            );
                            self.state
                                .out
                                .emit_instr_imm_reg("    andq", -(align as i64), "rax");
                        } else {
                            self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rax");
                        }
                        self.state.emit("    xorl %edx, %edx");
                    } else if self.state.is_i128_value(v.0) {
                        // 128-bit value in 16-byte stack slot
                        if std::env::var("CCC_TRACE_I128").is_ok() {
                            eprintln!(
                                "[I128] load_acc_pair value {} slot {} rsp_frame_size={}",
                                v.0, slot.0, self.state.out.rsp_frame_size
                            );
                        }
                        self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
                        self.state
                            .out
                            .emit_instr_rbp_reg("    movq", slot.0 + 8, "rdx");
                    } else {
                        // Non-i128 value (e.g. shift amount): load width-typed,
                        // zero-extend into rax, clear rdx.
                        // Check register allocation first, since register-allocated
                        // values may not have their stack slot written.
                        if let Some(&reg) = self.reg_assignments.get(&v.0) {
                            let reg_name = phys_reg_name(reg);
                            self.state
                                .out
                                .emit_instr_reg_reg("    movq", reg_name, "rax");
                        } else if matches!(
                            self.value_types.get(&v.0),
                            Some(IrType::I8)
                                | Some(IrType::U8)
                                | Some(IrType::I16)
                                | Some(IrType::U16)
                                | Some(IrType::I32)
                                | Some(IrType::U32)
                                | Some(IrType::F32)
                        ) {
                            // Width-consistent load (see value_to_reg): the
                            // store wrote movb/movw/movl or movq; loading the
                            // matching width is correct in both cases.
                            let ty = self.value_types.get(&v.0).copied().unwrap_or(IrType::I64);
                            let needs_sext = ty == IrType::I32
                                && !self.needs_sext_values.is_empty()
                                && self.needs_sext_values.contains(&v.0);
                            match ty {
                                IrType::I8 => {
                                    self.state
                                        .out
                                        .emit_instr_rbp_reg("    movsbq", slot.0, "rax")
                                }
                                IrType::U8 => {
                                    self.state
                                        .out
                                        .emit_instr_rbp_reg("    movzbl", slot.0, "eax")
                                }
                                IrType::I16 => {
                                    self.state
                                        .out
                                        .emit_instr_rbp_reg("    movswq", slot.0, "rax")
                                }
                                IrType::U16 => {
                                    self.state
                                        .out
                                        .emit_instr_rbp_reg("    movzwl", slot.0, "eax")
                                }
                                _ if needs_sext => {
                                    self.state
                                        .out
                                        .emit_instr_rbp_reg("    movslq", slot.0, "rax")
                                }
                                _ => self.state.out.emit_instr_rbp_reg("    movl", slot.0, "eax"),
                            }
                        } else {
                            self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rax");
                        }
                        self.state.emit("    xorl %edx, %edx");
                    }
                } else {
                    // No stack slot: check register allocation
                    if let Some(&reg) = self.reg_assignments.get(&v.0) {
                        let reg_name = phys_reg_name(reg);
                        self.state
                            .out
                            .emit_instr_reg_reg("    movq", reg_name, "rax");
                        self.state.emit("    xorl %edx, %edx");
                    } else {
                        self.state.emit("    xorl %eax, %eax");
                        self.state.emit("    xorl %edx, %edx");
                    }
                }
            }
        }
    }

    /// Store %rax:%rdx (128-bit) to a value's 16-byte stack slot.
    pub(super) fn store_rax_rdx_to(&mut self, dest: &Value) {
        if let Some(slot) = self.state.get_slot(dest.0) {
            if std::env::var("CCC_TRACE_I128").is_ok() {
                eprintln!(
                    "[I128] store_rax_rdx_to: value {} -> slot {}",
                    dest.0, slot.0
                );
            }
            self.state.out.emit_instr_reg_rbp("    movq", "rax", slot.0);
            self.state
                .out
                .emit_instr_reg_rbp("    movq", "rdx", slot.0 + 8);
        } else if std::env::var("CCC_TRACE_I128").is_ok() {
            eprintln!(
                "[I128] store_rax_rdx_to: value {} has NO slot — pair left in rax:rdx (may be clobbered)",
                dest.0
            );
        }
        // rax holds only the low 64 bits of an i128, not a valid scalar IR value.
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    /// Get the store instruction mnemonic for a given type.
    pub(super) fn mov_store_for_type(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "movb",
            IrType::I16 | IrType::U16 => "movw",
            IrType::I32 | IrType::U32 | IrType::F32 => "movl",
            _ => "movq",
        }
    }

    /// Get the load instruction mnemonic for a given type.
    pub(super) fn mov_load_for_type(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 => "movsbq",
            IrType::U8 => "movzbl", // movzbl zero-extends to 64-bit implicitly (saves REX vs movzbq)
            IrType::I16 => "movswq",
            IrType::U16 => "movzwl", // movzwl zero-extends to 64-bit implicitly (saves REX vs movzwq)
            IrType::I32 => "movslq",
            IrType::U32 | IrType::F32 => "movl", // movl zero-extends to 64-bit implicitly
            _ => "movq",
        }
    }

    /// Load instruction for a value, using movl instead of movslq for I32 when
    /// the value doesn't need sign extension (not in needs_sext_values).
    pub(super) fn mov_load_for_value(&self, ty: IrType, dest_id: u32) -> &'static str {
        if ty == IrType::I32
            && !self.needs_sext_values.is_empty()
            && !self.needs_sext_values.contains(&dest_id)
        {
            return "movl";
        }
        if ty == IrType::I32 && self.skip_i32_sext {
            return "movl";
        }
        Self::mov_load_for_type(ty)
    }

    /// Destination register for a value load. When using movl (I32 without sext),
    /// uses the 32-bit register name (eax) for implicit zero-extension.
    pub(super) fn load_dest_reg_for_value(&self, ty: IrType, dest_id: u32) -> &'static str {
        if ty == IrType::I32 && self.mov_load_for_value(ty, dest_id) == "movl" {
            return "eax";
        }
        Self::load_dest_reg(ty)
    }

    /// Destination register for loads. The width must match the mnemonic
    /// chosen by `mov_load_for_type`, not the source type:
    ///
    ///   * I8   -> `movsbq` (sign-extend 8 -> 64) takes a 64-bit dest (`%rax`);
    ///   * U8   -> `movzbl` (zero-extend 8 -> 32) takes a 32-bit dest (`%eax`);
    ///   * I16  -> `movswq` (sign-extend 16 -> 64) takes a 64-bit dest (`%rax`);
    ///   * U16  -> `movzwl` (zero-extend 16 -> 32) takes a 32-bit dest (`%eax`);
    ///   * U32/F32 -> `movl` zero-extends to 64 implicitly -> 32-bit dest (`%eax`);
    ///   * else  -> `movq` -> 64-bit dest (`%rax`).
    ///
    /// GNU as rejects `movzbl %sil, %al` / `movsbq %sil, %al` ("incorrect
    /// register ... used with suffix"); this is the same mnemonic-driven
    /// width contract as `asm_load_dest_reg` in inline_asm.rs. It also keeps
    /// `%rax` zero/sign-extended for the `store_rax_to` spill that follows.
    pub(super) fn load_dest_reg(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::I16 => "%rax", // movsbq/movswq -> 64-bit dest
            IrType::U8 | IrType::U16 => "%eax", // movzbl/movzwl -> 32-bit dest
            IrType::U32 | IrType::F32 => "%eax",
            _ => "%rax",
        }
    }

    /// Map base register name + type to sized sub-register.
    pub(super) fn reg_for_type(base_reg: &str, ty: IrType) -> &'static str {
        let (r8, r16, r32, r64) = match base_reg {
            "rax" => ("al", "ax", "eax", "rax"),
            "rcx" => ("cl", "cx", "ecx", "rcx"),
            "rdx" => ("dl", "dx", "edx", "rdx"),
            "rdi" => ("dil", "di", "edi", "rdi"),
            "rsi" => ("sil", "si", "esi", "rsi"),
            "r8" => ("r8b", "r8w", "r8d", "r8"),
            "r9" => ("r9b", "r9w", "r9d", "r9"),
            _ => return "rax",
        };
        match ty {
            IrType::I8 | IrType::U8 => r8,
            IrType::I16 | IrType::U16 => r16,
            IrType::I32 | IrType::U32 | IrType::F32 => r32,
            _ => r64,
        }
    }

    /// Get the type suffix for lock-prefixed instructions (b, w, l, q).
    pub(super) fn type_suffix(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 => "b",
            IrType::I16 | IrType::U16 => "w",
            IrType::I32 | IrType::U32 => "l",
            _ => "q",
        }
    }

    /// Emit a cmpxchg-based loop for atomic sub/and/or/xor/nand.
    /// Expects: rax = operand val, rcx = ptr address.
    /// After: rax = old value.
    pub(super) fn emit_x86_atomic_op_loop(&mut self, ty: IrType, op: &str) {
        // Save val to rdi (using rdi instead of r8 to free r8 for register allocation)
        self.state.emit("    movq %rax, %rdi"); // rdi = val
                                                // Load old value
        let load_instr = Self::mov_load_for_type(ty);
        let load_dest = Self::load_dest_reg(ty);
        self.state
            .emit_fmt(format_args!("    {} (%rcx), {}", load_instr, load_dest));
        // Loop: rax = old, compute new = op(old, val), try cmpxchg
        let label_id = self.state.next_label_id();
        let loop_label = format!(".Latomic_loop_{}", label_id);
        self.state.out.emit_named_label(&loop_label);
        // rdx = rax (old)
        self.state.emit("    movq %rax, %rdx");
        // Apply operation: rdx = op(rdx, rdi)
        let size_suffix = Self::type_suffix(ty);
        let rdx_reg = Self::reg_for_type("rdx", ty);
        let r8_reg = match ty {
            IrType::I8 | IrType::U8 => "dil",
            IrType::I16 | IrType::U16 => "di",
            IrType::I32 | IrType::U32 => "edi",
            _ => "rdi",
        };
        match op {
            "sub" => self.state.emit_fmt(format_args!(
                "    sub{} %{}, %{}",
                size_suffix, r8_reg, rdx_reg
            )),
            "and" => self.state.emit_fmt(format_args!(
                "    and{} %{}, %{}",
                size_suffix, r8_reg, rdx_reg
            )),
            "or" => self.state.emit_fmt(format_args!(
                "    or{} %{}, %{}",
                size_suffix, r8_reg, rdx_reg
            )),
            "xor" => self.state.emit_fmt(format_args!(
                "    xor{} %{}, %{}",
                size_suffix, r8_reg, rdx_reg
            )),
            "nand" => {
                self.state.emit_fmt(format_args!(
                    "    and{} %{}, %{}",
                    size_suffix, r8_reg, rdx_reg
                ));
                self.state
                    .emit_fmt(format_args!("    not{} %{}", size_suffix, rdx_reg));
            }
            _ => {}
        }
        // Try cmpxchg: if [rcx] == rax (old), set [rcx] = rdx (new), else rax = [rcx]
        self.state.emit_fmt(format_args!(
            "    lock cmpxchg{} %{}, (%rcx)",
            size_suffix, rdx_reg
        ));
        self.state.out.emit_jcc_label("    jne", &loop_label);
        // rax = old value on success
    }

    /// Load i128 operands for binary ops: lhs → rax:rdx, rhs → rcx:rsi.
    pub(super) fn prep_i128_binop(&mut self, lhs: &Operand, rhs: &Operand) {
        self.operand_to_rax_rdx(lhs);
        self.state.emit("    pushq %rdx");
        self.state.emit("    pushq %rax");
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size += 16;
        }
        self.operand_to_rax_rdx(rhs);
        self.state.emit("    movq %rax, %rcx");
        self.state.emit("    movq %rdx, %rsi");
        self.state.emit("    popq %rax");
        self.state.emit("    popq %rdx");
        if self.state.out.use_rsp_addressing {
            self.state.out.rsp_frame_size -= 16;
        }
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    /// Load an operand value into any GP register (returned as string).
    /// Uses rcx as the scratch register.
    pub(super) fn operand_to_reg(&mut self, op: &Operand, reg: &str) {
        match op {
            Operand::Const(c) => match c {
                IrConst::I8(v) if *v == 0 => self
                    .state
                    .emit_fmt(format_args!("    xorl %{0}, %{0}", reg_name_to_32(reg))),
                IrConst::I16(v) if *v == 0 => self
                    .state
                    .emit_fmt(format_args!("    xorl %{0}, %{0}", reg_name_to_32(reg))),
                IrConst::I32(v) if *v == 0 => self
                    .state
                    .emit_fmt(format_args!("    xorl %{0}, %{0}", reg_name_to_32(reg))),
                IrConst::I64(0) => self
                    .state
                    .emit_fmt(format_args!("    xorl %{0}, %{0}", reg_name_to_32(reg))),
                // PF-16: non-negative immediates via movl (see site above).
                IrConst::I8(v) => {
                    let x = *v as i64;
                    if x >= 0 {
                        self.state.emit_fmt(format_args!(
                            "    movl ${}, %{}",
                            x,
                            reg_name_to_32(reg)
                        ));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movq ${}, %{}", x, reg));
                    }
                }
                IrConst::I16(v) => {
                    let x = *v as i64;
                    if x >= 0 {
                        self.state.emit_fmt(format_args!(
                            "    movl ${}, %{}",
                            x,
                            reg_name_to_32(reg)
                        ));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movq ${}, %{}", x, reg));
                    }
                }
                IrConst::I32(v) => {
                    let x = *v as i64;
                    if x >= 0 {
                        self.state.emit_fmt(format_args!(
                            "    movl ${}, %{}",
                            x,
                            reg_name_to_32(reg)
                        ));
                    } else {
                        self.state
                            .emit_fmt(format_args!("    movq ${}, %{}", x, reg));
                    }
                }
                IrConst::I64(v) => {
                    if *v >= 0 && *v <= i32::MAX as i64 {
                        self.state
                            .out
                            .emit_instr_imm_reg("    movl", *v, reg_name_to_32(reg));
                    } else if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                        self.state.out.emit_instr_imm_reg("    movq", *v, reg);
                    } else {
                        self.state.out.emit_instr_imm_reg("    movabsq", *v, reg);
                    }
                }
                _ => self
                    .state
                    .emit_fmt(format_args!("    xorl %{0}, %{0}", reg_name_to_32(reg))),
            },
            Operand::Value(v) => {
                self.value_to_reg(v, reg);
            }
        }
    }

    /// Extract an immediate integer value from an operand.
    /// Used for SSE/AES instructions that require compile-time immediate operands.
    pub(super) fn operand_to_imm_i64(&self, op: &Operand) -> i64 {
        match op {
            Operand::Const(c) => match c {
                IrConst::I8(v) => *v as i64,
                IrConst::I16(v) => *v as i64,
                IrConst::I32(v) => *v as i64,
                IrConst::I64(v) => *v,
                _ => 0,
            },
            Operand::Value(_) => {
                // TODO: this shouldn't happen for compile-time immediate arguments;
                // the frontend should always fold these to constants.
                0
            }
        }
    }

    /// Emit comment annotations for callee-saved registers listed in inline asm
    /// clobber lists. The peephole pass's `eliminate_unused_callee_saves` scans
    /// function bodies for textual register references (e.g., "%rbx") to decide
    /// whether a callee-saved register save/restore can be eliminated. Without
    /// these annotations, an inline asm that clobbers a callee-saved register
    /// (but doesn't mention it in the emitted assembly text) would have its
    /// save/restore incorrectly removed.
    pub(super) fn emit_callee_saved_clobber_annotations(&mut self, clobbers: &[String]) {
        for clobber in clobbers {
            let clobber = crate::backend::inline_asm::normalize_inline_asm_reg_name(clobber);
            let reg_name = match clobber {
                "rbx" | "ebx" | "bx" | "bl" | "bh" => Some("%rbx"),
                "r12" | "r12d" | "r12w" | "r12b" => Some("%r12"),
                "r13" | "r13d" | "r13w" | "r13b" => Some("%r13"),
                "r14" | "r14d" | "r14w" | "r14b" => Some("%r14"),
                "r15" | "r15d" | "r15w" | "r15b" => Some("%r15"),
                _ => None,
            };
            if let Some(reg) = reg_name {
                self.state
                    .emit_fmt(format_args!("    # asm clobber {}", reg));
            }
        }
    }

    /// LEA scale factor for multiply strength reduction.
    /// Returns the LEA scale factor for multipliers 3, 5, 9 (which decompose
    /// as reg + reg*2, reg + reg*4, reg + reg*8 respectively).
    pub(super) fn lea_scale_for_mul(imm: i64) -> Option<u8> {
        match imm {
            3 => Some(2),
            5 => Some(4),
            9 => Some(8),
            _ => None,
        }
    }

    /// Register-direct path for simple ALU ops (add/sub/and/or/xor/mul).
    pub(super) fn emit_alu_reg_direct(
        &mut self,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        dest_phys: PhysReg,
        use_32bit: bool,
        is_unsigned: bool,
        dest_value_id: u32,
    ) {
        let dest_name = phys_reg_name(dest_phys);
        // Safety: verify the 32-bit name is valid. The "rbpd" bug appears to be
        // caused by some codegen path concatenating the 64-bit name with "d" suffix
        // instead of using the proper 32-bit register name.
        let dest_name_32 = phys_reg_name_32(dest_phys);
        debug_assert!(
            !dest_name_32.contains("rbp") || dest_name_32 == "ebp",
            "unexpected 32-bit name for rbp: {}",
            dest_name_32
        );

        // Immediate form
        if let Some(imm) = Self::const_as_imm32(rhs) {
            self.operand_to_callee_reg(lhs, dest_phys);
            if op == IrBinOp::Mul {
                // LEA strength reduction: replace imul by 3/5/9 with lea.
                if let Some(scale) = Self::lea_scale_for_mul(imm) {
                    if use_32bit {
                        self.state.emit_fmt(format_args!(
                            "    leal (%{}, %{}, {}), %{}",
                            dest_name_32, dest_name_32, scale, dest_name_32
                        ));
                        self.emit_sext32_for_value(
                            dest_name_32,
                            dest_name,
                            is_unsigned,
                            dest_value_id,
                        );
                    } else {
                        self.state.emit_fmt(format_args!(
                            "    leaq (%{}, %{}, {}), %{}",
                            dest_name, dest_name, scale, dest_name
                        ));
                    }
                } else if use_32bit {
                    // Encode u32 constants as signed imm32 (same bits).
                    let imm32 = imm as i32 as i64;
                    self.state.emit_fmt(format_args!(
                        "    imull ${}, %{}, %{}",
                        imm32, dest_name_32, dest_name_32
                    ));
                    self.emit_sext32_for_value(dest_name_32, dest_name, is_unsigned, dest_value_id);
                } else {
                    self.state.emit_fmt(format_args!(
                        "    imulq ${}, %{}, %{}",
                        imm, dest_name, dest_name
                    ));
                }
            } else {
                let mnemonic = alu_mnemonic(op);
                if use_32bit {
                    self.state.emit_fmt(format_args!(
                        "    {}l ${}, %{}",
                        mnemonic, imm, dest_name_32
                    ));
                    self.emit_sext32_for_value(dest_name_32, dest_name, is_unsigned, dest_value_id);
                } else {
                    self.state
                        .emit_fmt(format_args!("    {}q ${}, %{}", mnemonic, imm, dest_name));
                }
            }
            self.state.reg_cache.invalidate_acc();
            return;
        }

        // Register-register form
        // Integer ALU reg-direct requires GPR names; an XMM-homed operand
        // (bit-punned float words after hidden_proto math wrappers inline)
        // must take the accumulator fallback, which handles the xmm->GPR
        // move (phys_reg_name_32(xmm) is unreachable!()).
        let rhs_phys = self.operand_reg(rhs).filter(|r| !is_xmm_reg(*r));
        let rhs_conflicts = rhs_phys.is_some_and(|r| r.0 == dest_phys.0);
        let (rhs_reg_name, rhs_reg_name_32): (String, String) = if rhs_conflicts {
            // rhs is in dest register — we need to save rhs to %rax before loading lhs.
            // But if lhs is in the accumulator cache, we must save it to dest first,
            // because operand_to_rax(rhs) would clobber %rax.
            let lhs_in_acc = match lhs {
                Operand::Value(v) => {
                    let is_alloca = self.state.is_alloca(v.0);
                    self.state.reg_cache.acc_has(v.0, is_alloca)
                }
                _ => false,
            };
            if lhs_in_acc {
                // RA-09: lhs is in %rax and rhs's home IS the dest register.
                // The old code saved lhs into dest (`movq %rax,%dest`) and then
                // re-read "rhs" from dest (`movq %dest,%rax`) — but that first
                // move clobbered rhs's value, so the re-read returned lhs, and
                // `op %rax,%dest` computed `lhs op lhs` (a self-op / miscompile,
                // e.g. base.c `xorl %ebx,%ebx` when 131/128/132 all homed rbx).
                // Both operands are already staged correctly: lhs in %rax, rhs
                // in dest. The common tail `op %rax, %dest` then computes
                // `rhs op lhs` into dest, which equals `lhs op rhs` for every
                // commutative ALU op (Add/And/Or/Xor/Mul). Sub is non-commutative
                // and must preserve operand order (lhs - rhs), so it is staged
                // through %rax below before the tail.
                if op == IrBinOp::Sub {
                    let rhs_name = phys_reg_name(rhs_phys.unwrap());
                    let rhs_name_32 = phys_reg_name_32(rhs_phys.unwrap());
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    subl %{}, %eax", rhs_name_32));
                        self.state
                            .emit_fmt(format_args!("    movl %eax, %{}", dest_name_32));
                        self.emit_sext32_for_value(
                            dest_name_32,
                            dest_name,
                            is_unsigned,
                            dest_value_id,
                        );
                    } else {
                        self.state
                            .emit_fmt(format_args!("    subq %{}, %rax", rhs_name));
                        self.state
                            .emit_fmt(format_args!("    movq %rax, %{}", dest_name));
                    }
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
                // Commutative: leave lhs in %rax, rhs in dest. The tail emits
                // `op %rax, %dest`.
            } else {
                self.operand_to_rax(rhs);
                self.operand_to_callee_reg(lhs, dest_phys);
            }
            ("rax".to_string(), "eax".to_string())
        } else {
            self.operand_to_callee_reg(lhs, dest_phys);
            if let Some(rhs_phys) = rhs_phys {
                (
                    phys_reg_name(rhs_phys).to_string(),
                    phys_reg_name_32(rhs_phys).to_string(),
                )
            } else {
                self.operand_to_rax(rhs);
                ("rax".to_string(), "eax".to_string())
            }
        };

        if op == IrBinOp::Mul {
            if use_32bit {
                self.state
                    .out
                    .emit_instr_reg_reg("    imull", &rhs_reg_name_32, dest_name_32);
                self.emit_sext32_for_value(dest_name_32, dest_name, is_unsigned, dest_value_id);
            } else {
                self.state
                    .out
                    .emit_instr_reg_reg("    imulq", &rhs_reg_name, dest_name);
            }
        } else {
            let mnemonic = alu_mnemonic(op);
            if use_32bit {
                // 32-bit ALU ops (addl/andl/xorl/...) are shorter and faster than
                // their 64-bit forms. Add/Sub already took this path; And/Or/Xor/Mul
                // previously emitted `q`-suffix ops on zero/sign-extended homes,
                // which is semantically correct but wasteful. All simple ALU ops
                // have identical low-32-bit results, and the per-value
                // sign-extension discipline (emit_sext32_for_value) keeps the
                // 64-bit home correctly extended for signed I32 values.
                self.state.emit_fmt(format_args!(
                    "    {}l %{}, %{}",
                    mnemonic, rhs_reg_name_32, dest_name_32
                ));
                self.emit_sext32_for_value(dest_name_32, dest_name, is_unsigned, dest_value_id);
            } else {
                self.state.emit_fmt(format_args!(
                    "    {}q %{}, %{}",
                    mnemonic, rhs_reg_name, dest_name
                ));
            }
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Register-direct path for shift operations.
    pub(super) fn emit_shift_reg_direct(
        &mut self,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        dest_phys: PhysReg,
        use_32bit: bool,
        is_unsigned: bool,
        dest_value_id: u32,
    ) {
        let dest_name = phys_reg_name(dest_phys);
        let dest_name_32 = phys_reg_name_32(dest_phys);
        let (mnem32, mnem64) = shift_mnemonic(op);

        if let Some(imm) = Self::const_as_imm32(rhs) {
            self.operand_to_callee_reg(lhs, dest_phys);
            if use_32bit {
                let shift_amount = (imm as u32) & 31;
                self.state.emit_fmt(format_args!(
                    "    {} ${}, %{}",
                    mnem32, shift_amount, dest_name_32
                ));
                if !is_unsigned && matches!(op, IrBinOp::Shl | IrBinOp::AShr) {
                    self.emit_sext32_for_value(dest_name_32, dest_name, false, dest_value_id);
                }
            } else {
                let shift_amount = (imm as u64) & 63;
                self.state.emit_fmt(format_args!(
                    "    {} ${}, %{}",
                    mnem64, shift_amount, dest_name
                ));
            }
        } else {
            let rhs_conflicts = self.operand_reg(rhs).is_some_and(|r| r.0 == dest_phys.0);
            if rhs_conflicts {
                self.operand_to_rcx(rhs);
                self.operand_to_callee_reg(lhs, dest_phys);
            } else {
                self.operand_to_callee_reg(lhs, dest_phys);
                self.operand_to_rcx(rhs);
            }
            if use_32bit {
                self.state
                    .emit_fmt(format_args!("    {} %cl, %{}", mnem32, dest_name_32));
                if !is_unsigned && matches!(op, IrBinOp::Shl | IrBinOp::AShr) {
                    self.emit_sext32_for_value(dest_name_32, dest_name, false, dest_value_id);
                }
            } else {
                self.state
                    .emit_fmt(format_args!("    {} %cl, %{}", mnem64, dest_name));
            }
        }
        self.state.reg_cache.invalidate_acc();
    }

    /// Accumulator-based path: try immediate optimizations first.
    /// Returns true if handled.
    pub(super) fn try_emit_acc_immediate(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        use_32bit: bool,
        is_unsigned: bool,
    ) -> bool {
        // Immediate ALU ops
        if matches!(
            op,
            IrBinOp::Add | IrBinOp::Sub | IrBinOp::And | IrBinOp::Or | IrBinOp::Xor
        ) {
            if let Some(imm) = Self::const_as_imm32_typed(rhs, use_32bit) {
                self.operand_to_rax(lhs);
                let mnemonic = alu_mnemonic(op);
                // The l-form is REQUIRED (not just preferred) when the
                // immediate only exists as a 32-bit bit pattern
                // (i32::MAX < imm <= u32::MAX): the q-form sign-extends its
                // imm32, so `xorq $0x80000000` is unencodable — the old
                // assembler silently turned it into XOR with
                // 0xFFFFFFFF80000000 (wrong 33 bits). After the l-form the
                // canonical 64-bit slot value is restored exactly like the
                // Add/Sub path: cltq for signed, implicit zero-extension for
                // unsigned.
                let needs_l_form = imm > i32::MAX as i64;
                if use_32bit && (matches!(op, IrBinOp::Add | IrBinOp::Sub) || needs_l_form) {
                    self.state
                        .emit_fmt(format_args!("    {}l ${}, %eax", mnemonic, imm));
                    if !is_unsigned {
                        self.state.emit("    cltq");
                    }
                } else {
                    self.state
                        .emit_fmt(format_args!("    {}q ${}, %rax", mnemonic, imm));
                }
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
                return true;
            }
        }

        // Immediate multiply
        if op == IrBinOp::Mul {
            if let Some(imm) = Self::const_as_imm32_typed(rhs, use_32bit) {
                self.operand_to_rax(lhs);
                // LEA strength reduction: x*3/5/9 → lea (%rax, %rax, scale), %rax.
                // lea has 1-cycle latency vs 3 cycles for imul on modern x86.
                if let Some(scale) = Self::lea_scale_for_mul(imm) {
                    if use_32bit {
                        self.state
                            .emit_fmt(format_args!("    leal (%eax, %eax, {}), %eax", scale));
                        if !is_unsigned {
                            self.state.emit("    cltq");
                        }
                    } else {
                        self.state
                            .emit_fmt(format_args!("    leaq (%rax, %rax, {}), %rax", scale));
                    }
                } else if use_32bit {
                    let imm32 = imm as i32 as i64;
                    self.state
                        .emit_fmt(format_args!("    imull ${}, %eax, %eax", imm32));
                    if !is_unsigned {
                        self.state.emit("    cltq");
                    }
                } else {
                    self.state
                        .emit_fmt(format_args!("    imulq ${}, %rax, %rax", imm));
                }
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
                return true;
            }
        }

        // Immediate shift
        if matches!(op, IrBinOp::Shl | IrBinOp::AShr | IrBinOp::LShr) {
            if let Some(imm) = Self::const_as_imm32(rhs) {
                self.operand_to_rax(lhs);
                let (mnem32, mnem64) = shift_mnemonic(op);
                if use_32bit {
                    let shift_amount = (imm as u32) & 31;
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %eax", mnem32, shift_amount));
                    if !is_unsigned && matches!(op, IrBinOp::Shl | IrBinOp::AShr) {
                        self.state.emit("    cltq");
                    }
                } else {
                    let shift_amount = (imm as u64) & 63;
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %rax", mnem64, shift_amount));
                }
                self.state.reg_cache.invalidate_acc();
                self.store_rax_to(dest);
                return true;
            }
        }

        false
    }

    /// Helper to load va_list pointer into %rcx for va_arg operations.
    pub(super) fn load_va_list_ptr_to_rcx(&mut self, va_list_ptr: &Value) {
        if let Some(&reg) = self.reg_assignments.get(&va_list_ptr.0) {
            let reg_name = phys_reg_name(reg);
            self.state
                .out
                .emit_instr_reg_reg("    movq", reg_name, "rcx");
        } else if let Some(slot) = self.state.get_slot(va_list_ptr.0) {
            if self.state.is_alloca(va_list_ptr.0) {
                self.state.out.emit_instr_rbp_reg("    leaq", slot.0, "rcx");
            } else {
                self.state.out.emit_instr_rbp_reg("    movq", slot.0, "rcx");
            }
        }
    }
}

pub(super) const X86_ARG_REGS: [&str; 6] = ["rdi", "rsi", "rdx", "rcx", "r8", "r9"];

/// Collect all vreg IDs from a MachInst, separating into all_vregs and def_vregs.
fn collect_vregs_in_inst(
    inst: &super::machinst::MachInst,
    all: &mut crate::common::fx_hash::FxHashSet<u32>,
    defs: &mut crate::common::fx_hash::FxHashSet<u32>,
) {
    use super::machinst::{MachInst, MachOperand, MachReg};
    let use_reg = |r: &MachReg, all: &mut crate::common::fx_hash::FxHashSet<u32>| {
        if let MachReg::Vreg(id) = r {
            all.insert(*id);
        }
    };
    let use_op = |o: &MachOperand, all: &mut crate::common::fx_hash::FxHashSet<u32>| match o {
        MachOperand::Reg(r) => {
            if let MachReg::Vreg(id) = r {
                all.insert(*id);
            }
        }
        MachOperand::Mem { base, .. } => {
            if let MachReg::Vreg(id) = base {
                all.insert(*id);
            }
        }
        MachOperand::MemIndex { base, index, .. } => {
            if let MachReg::Vreg(id) = base {
                all.insert(*id);
            }
            if let MachReg::Vreg(id) = index {
                all.insert(*id);
            }
        }
        _ => {}
    };
    let def_reg = |r: &MachReg,
                   all: &mut crate::common::fx_hash::FxHashSet<u32>,
                   defs: &mut crate::common::fx_hash::FxHashSet<u32>| {
        if let MachReg::Vreg(id) = r {
            all.insert(*id);
            defs.insert(*id);
        }
    };
    match inst {
        MachInst::Mov { src, dst, .. } => {
            use_op(src, all);
            if let MachOperand::Reg(r) = dst {
                def_reg(r, all, defs);
            } else {
                use_op(dst, all);
            }
        }
        MachInst::Alu { src, dst, .. } => {
            use_op(src, all);
            use_reg(dst, all);
            def_reg(dst, all, defs);
        }
        MachInst::Imul3 { src, dst, .. } => {
            use_reg(src, all);
            def_reg(dst, all, defs);
        }
        MachInst::Neg { dst, .. } | MachInst::Not { dst, .. } => {
            use_reg(dst, all);
            def_reg(dst, all, defs);
        }
        MachInst::Shift { amount, dst, .. } => {
            use_op(amount, all);
            use_reg(dst, all);
            def_reg(dst, all, defs);
        }
        MachInst::Lea {
            base, index, dst, ..
        } => {
            use_reg(base, all);
            if let Some((r, _)) = index {
                use_reg(r, all);
            }
            def_reg(dst, all, defs);
        }
        MachInst::Div { divisor, .. } => {
            use_op(divisor, all);
        }
        MachInst::Cmp { lhs, rhs, .. } | MachInst::Test { lhs, rhs, .. } => {
            use_op(lhs, all);
            use_op(rhs, all);
        }
        MachInst::SetCC { dst, .. } => {
            def_reg(dst, all, defs);
        }
        MachInst::Movzx { src, dst, .. } | MachInst::Movsx { src, dst, .. } => {
            use_op(src, all);
            def_reg(dst, all, defs);
        }
        MachInst::Cmov { src, dst, .. } => {
            use_op(src, all);
            use_reg(dst, all);
            def_reg(dst, all, defs);
        }
        _ => {}
    }
}

/// Peephole: fold store followed by immediate reload when the slot is re-written later.
fn fold_spill_relays(insts: &mut Vec<super::machinst::MachInst>) {
    use super::machinst::{MachInst, MachOperand};
    let mut last_write: crate::common::fx_hash::FxHashMap<i64, usize> =
        crate::common::fx_hash::FxHashMap::default();
    for (idx, inst) in insts.iter().enumerate() {
        if let MachInst::Mov {
            dst: MachOperand::StackSlot(s),
            ..
        } = inst
        {
            last_write.insert(*s, idx);
        }
    }
    let mut i = 0;
    while i + 1 < insts.len() {
        if let MachInst::Mov {
            src: src1,
            dst: MachOperand::StackSlot(s1),
            size: sz1,
        } = &insts[i]
        {
            if let MachInst::Mov {
                src: MachOperand::StackSlot(s2),
                dst: dst2,
                size: sz2,
            } = &insts[i + 1]
            {
                if s1 == s2 && sz1 == sz2 && matches!(dst2, MachOperand::Reg(_)) {
                    if let Some(&last_idx) = last_write.get(s1) {
                        if last_idx > i {
                            let folded = MachInst::Mov {
                                src: src1.clone(),
                                dst: dst2.clone(),
                                size: *sz1,
                            };
                            insts[i] = folded;
                            insts.remove(i + 1);
                            last_write.clear();
                            for (idx, inst) in insts.iter().enumerate() {
                                if let MachInst::Mov {
                                    dst: MachOperand::StackSlot(s),
                                    ..
                                } = inst
                                {
                                    last_write.insert(*s, idx);
                                }
                            }
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }
}

/// Resolve stack-only vregs in a MachInst: replace MachReg::Vreg(id) with the
/// value's stack slot as a memory operand. For instructions where the vreg is a
/// dst (two-address ALU), we need to use rax as scratch: load→operate→store.
fn resolve_stack_vregs(
    inst: &super::machinst::MachInst,
    ra: &FxHashMap<u32, PhysReg>,
    state: &CodegenState,
) -> super::machinst::MachInst {
    use super::machinst::{MachInst, MachOperand, MachReg, OpSize};

    // Helper: resolve a MachReg. If it's a vreg without a register, keep it as-is
    // (the has_unresolvable_vreg check will catch it and abandon the buffer).
    let resolve_reg = |r: &MachReg| -> MachReg { *r };

    // WIDTH SOUNDNESS: a spilled vreg's slot is 4 bytes (small slot) or 8
    // bytes. Substituting it into an instruction whose size is WIDER than
    // the slot makes the emitter read bytes past the slot — e.g.
    // `cmpq $0, 28(%rsp)` on a 4-byte slot pulls the adjacent slot's
    // garbage into the high 32 bits (kernel lib/zstd
    // ZSTD_decodeLiteralsBlock: the 64-bit `!lhlCode` predicate read
    // `flags` from the neighboring slot and took the wrong HUF dispatch).
    // A register-form consumer is safe — a `movl` reload zero-extends —
    // but memory-operand substitution silently loses that extension.
    // Refuse the substitution when the instruction size exceeds the slot
    // width: the leftover Vreg trips has_unresolvable_vreg and the
    // buffered window falls back to the default (accumulator) path, which
    // loads every value at its true width.
    let slot_fits = |id: &u32, inst_size: OpSize| -> bool {
        if state.is_small_slot(*id) {
            // 4-byte slot: only 8/16/32-bit reads are in-bounds.
            matches!(inst_size, OpSize::S8 | OpSize::S16 | OpSize::S32)
        } else {
            // 8-byte slot: every instruction width reads in-bounds.
            true
        }
    };

    // Helper: resolve a MachOperand — replace Vreg with StackSlot if possible.
    let resolve_op = |op: &MachOperand, inst_size: OpSize| -> MachOperand {
        match op {
            MachOperand::Reg(MachReg::Vreg(id)) if !ra.contains_key(id) => {
                if let Some(slot) = state.get_slot(*id) {
                    if slot_fits(id, inst_size) {
                        MachOperand::StackSlot(slot.0)
                    } else {
                        op.clone()
                    }
                } else {
                    op.clone()
                }
            }
            MachOperand::Mem {
                base: MachReg::Vreg(id),
                offset,
            } if !ra.contains_key(id) => {
                // Can't have a stack-slot-based-memory operand. Keep as vreg.
                op.clone()
            }
            _ => op.clone(),
        }
    };

    match inst {
        MachInst::Mov {
            src: MachOperand::AllocaAddr(id),
            dst,
            size,
        } => {
            // AllocaAddr: resolve to StackSlot but keep AllocaAddr tag so
            // the emitter knows to use leaq instead of movq.
            if let Some(slot) = state.get_slot(*id) {
                // Emit as: Mov { src: AllocaAddr but with resolved slot, dst }
                // We encode the slot in a special way the emitter recognizes.
                // Actually, just resolve to a Lea with StackSlot-based addressing.
                // The emitter handles StackSlot with use_rsp_addressing.
                // Produce: Raw("    leaq slot(%rbp), %reg") with correct addressing.
                if let MachOperand::Reg(r) = dst {
                    let dst_name = super::machinst_emit::reg_name_pub(*r);
                    if state.out.use_rsp_addressing {
                        let rsp_off = state.out.rsp_frame_size + slot.0;
                        return MachInst::Raw(format!("    leaq {}(%rsp), %{}", rsp_off, dst_name));
                    } else {
                        return MachInst::Raw(format!("    leaq {}(%rbp), %{}", slot.0, dst_name));
                    }
                }
                MachInst::Mov {
                    src: MachOperand::StackSlot(slot.0),
                    dst: resolve_op(dst, *size),
                    size: *size,
                }
            } else {
                inst.clone()
            }
        }
        MachInst::Mov { src, dst, size } => MachInst::Mov {
            src: resolve_op(src, *size),
            dst: resolve_op(dst, *size),
            size: *size,
        },
        // FMov never carries vregs: the lowering admits only xmm-homed
        // values and GPR/slot addressing, all pre-colored. Resolve an FMov
        // to itself; should a Vreg ever slip in, has_unresolvable_vreg
        // rejects the window and the default path replays it. (Resolving to
        // StackSlot here could create the mem-to-mem shape the SSE encoder
        // cannot express.)
        MachInst::FMov { .. } => inst.clone(),
        // FAlu's src2 may legitimately be a slot-homed Vreg: resolve it to
        // the StackSlot memory operand (the VEX form folds one memory
        // source). src1/dst are xmm homes by construction; a Vreg there
        // survives and trips the fallback.
        MachInst::FAlu {
            op,
            src2,
            src1,
            dst,
            size,
        } => MachInst::FAlu {
            op: *op,
            src2: resolve_op(src2, *size),
            src1: *src1,
            dst: *dst,
            size: *size,
        },
        MachInst::Alu { op, src, dst, size } => {
            if let MachReg::Vreg(id) = dst {
                if !ra.contains_key(id) {
                    // dst is on the stack — can't do two-address ALU with stack dest.
                    // Keep as-is; the has_unresolvable_vreg check will reject
                    // and fall back to existing accumulator codegen.
                    return inst.clone();
                }
            }
            MachInst::Alu {
                op: *op,
                src: resolve_op(src, *size),
                dst: *dst,
                size: *size,
            }
        }
        MachInst::Movzx {
            src,
            dst,
            from_size,
            to_size,
        } => {
            // movzx/movsx accept a memory source: a slot-backed (spilled) src
            // vreg can be resolved to StackSlot directly. The emitter formats
            // the src through fmt_operand, so `movzbl slot, %eax` is emitted.
            // The memory read is `from_size` bytes wide, which never exceeds
            // the slot (S8/S16/S32 reads of a 4-byte small slot read the
            // stored low bytes, exactly the value's bits). A slot-backed dst
            // stays a vreg and is caught by
            // has_unresolvable_vreg -> default-path fallback.
            let src_resolved = match src {
                MachOperand::Reg(MachReg::Vreg(id)) if !ra.contains_key(id) => {
                    if let Some(slot) = state.get_slot(*id) {
                        if slot_fits(id, *from_size) {
                            MachOperand::StackSlot(slot.0)
                        } else {
                            src.clone()
                        }
                    } else {
                        src.clone()
                    }
                }
                _ => src.clone(),
            };
            MachInst::Movzx {
                src: src_resolved,
                dst: *dst,
                from_size: *from_size,
                to_size: *to_size,
            }
        }
        MachInst::Movsx {
            src,
            dst,
            from_size,
            to_size,
        } => {
            let src_resolved = match src {
                MachOperand::Reg(MachReg::Vreg(id)) if !ra.contains_key(id) => {
                    if let Some(slot) = state.get_slot(*id) {
                        if slot_fits(id, *from_size) {
                            MachOperand::StackSlot(slot.0)
                        } else {
                            src.clone()
                        }
                    } else {
                        src.clone()
                    }
                }
                _ => src.clone(),
            };
            MachInst::Movsx {
                src: src_resolved,
                dst: *dst,
                from_size: *from_size,
                to_size: *to_size,
            }
        }
        MachInst::Cmp { lhs, rhs, size } => MachInst::Cmp {
            lhs: resolve_op(lhs, *size),
            rhs: resolve_op(rhs, *size),
            size: *size,
        },
        MachInst::Test { lhs, rhs, size } => MachInst::Test {
            lhs: resolve_op(lhs, *size),
            rhs: resolve_op(rhs, *size),
            size: *size,
        },
        MachInst::Cmov { cc, src, dst, size } => MachInst::Cmov {
            cc: *cc,
            src: resolve_op(src, *size),
            dst: *dst,
            size: *size,
        },
        MachInst::Shift {
            op,
            amount,
            dst,
            size,
        } => MachInst::Shift {
            op: *op,
            amount: resolve_op(amount, *size),
            dst: *dst,
            size: *size,
        },
        MachInst::Div {
            divisor,
            signed,
            size,
        } => MachInst::Div {
            divisor: resolve_op(divisor, *size),
            signed: *signed,
            size: *size,
        },
        // For instructions where the vreg is always a register (dst of SetCC,
        // Movzx, etc.), we can't replace with stack slot. Keep as-is.
        _ => inst.clone(),
    }
}

/// Check whether a MachInst still contains any virtual register.
///
/// Instruction selection converts every usable main-allocator GPR assignment
/// to `MachReg::Phys`. A remaining `Vreg` is therefore unresolved even when the
/// main assignment map contains the value (notably when that assignment is an
/// XMM register, which integer MachInst deliberately maps back to a Vreg).
/// Treating `ra.contains_key(id)` as resolution allowed `%vregN` to reach the
/// assembler and caused SQLite's MachInst build to fail on unary operations.
fn has_unresolvable_vreg(inst: &super::machinst::MachInst, _ra: &FxHashMap<u32, PhysReg>) -> bool {
    use super::machinst::{MachInst, MachOperand, MachReg};
    let check_reg = |r: &MachReg| -> bool { matches!(r, MachReg::Vreg(_)) };
    let check_op = |o: &MachOperand| -> bool {
        match o {
            MachOperand::Reg(r) => check_reg(r),
            MachOperand::Mem { base, .. } => check_reg(base),
            MachOperand::MemIndex { base, index, .. } => check_reg(base) || check_reg(index),
            // AllocaAddr is resolved to a Raw leaq only when a slot exists;
            // without one the emitter prints an error comment and the address
            // computation silently vanishes. Fall back instead.
            MachOperand::AllocaAddr(_) => true,
            _ => false,
        }
    };
    match inst {
        MachInst::Mov { src, dst, .. } => check_op(src) || check_op(dst),
        MachInst::FMov { src, dst, .. } => check_op(src) || check_op(dst),
        MachInst::FAlu {
            src2, src1, dst, ..
        } => check_op(src2) || check_reg(src1) || check_reg(dst),
        MachInst::Alu { src, dst, .. } => check_op(src) || check_reg(dst),
        MachInst::Imul3 { src, dst, .. } => check_reg(src) || check_reg(dst),
        MachInst::Neg { dst, .. } | MachInst::Not { dst, .. } => check_reg(dst),
        MachInst::Shift { amount, dst, .. } => check_op(amount) || check_reg(dst),
        MachInst::Lea {
            base, index, dst, ..
        } => {
            check_reg(base) || index.as_ref().map_or(false, |(r, _)| check_reg(r)) || check_reg(dst)
        }
        MachInst::Div { divisor, .. } => check_op(divisor),
        MachInst::Cmp { lhs, rhs, .. } | MachInst::Test { lhs, rhs, .. } => {
            check_op(lhs) || check_op(rhs)
        }
        MachInst::SetCC { dst, .. } => check_reg(dst),
        MachInst::Movzx { src, dst, .. } | MachInst::Movsx { src, dst, .. } => {
            check_op(src) || check_reg(dst)
        }
        MachInst::Cmov { src, dst, .. } => check_op(src) || check_reg(dst),
        _ => false,
    }
}

/// A value is MachInst-unsafe when it lives outside the GPR domain:
/// floats/XMM, vectors, i128/wide, or long double. MachInst's
/// lower_copy/lower_cast assume 64-bit GPR moves; touching such values
/// there miscompiles (float bit-copies read stale slots, i128 copies drop
/// the high half, vector slots get GPR-clobbered).
/// Destination home resolved for a [`Instruction::ParamRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamDst {
    /// Register-allocated GPR home (never rax/rcx/rbp/xmm — the
    /// umbrella guard plus the explicit id check reject those).
    Reg(PhysReg),
    /// Stack-slot home (spill or dedicated value slot).
    Slot(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamRefLoad {
    /// `movzbl`/`movzwl` — zero-extending load (8/16-bit sources).
    Zext(OpSize),
    /// `movsbq`/`movswq`/`movslq` — sign-extending load.
    Sext(OpSize),
    /// `movl` — raw 32-bit move (implicit zero-extend into the reg).
    Raw32,
    /// `movq` — raw 64-bit move.
    Raw64,
}

fn is_mi_unsafe_value(
    v: u32,
    reg_assignments: &FxHashMap<u32, PhysReg>,
    state: &CodegenState,
    value_types: &FxHashMap<u32, crate::common::types::IrType>,
) -> bool {
    // Accumulator assignments are point-constrained allocator locations. The
    // mature path consumes them directly; MachInst must not buffer them across
    // another instruction until its emitter models def/consume points.
    if state.is_accumulator_location(v)
        || state.is_i128_value(v)
        || state.vector_values.contains(&v)
    {
        return true;
    }
    if let Some(r) = reg_assignments.get(&v) {
        if r.0 >= 20 {
            return true; // XMM-assigned
        }
    }
    if let Some(ty) = value_types.get(&v) {
        if ty.is_float() || ty.is_long_double() || ty.is_128bit() {
            return true;
        }
    }
    false
}

impl X86Codegen {
    /// Validate the scalar-float typed subsets: FMov (Store/Load/Copy on
    /// F32/F64/D32/D64 whose values are all xmm-homed (xmm2..xmm15) and
    /// whose addressing is a direct alloca slot or a GPR-held pointer) and
    /// FAlu (Add/Sub/Mul/Div with an xmm-homed dest and xmm/slot-homed
    /// sources in the shapes the VEX three-operand form accepts).
    ///
    /// This is the emit-side twin of the isel float arms; both sides check
    /// the same subset so the typed lowering can never see a shape it does
    /// not model. Everything else (slot-homed floats needing the xmm scratch
    /// relay, constants, xmm-based pointers, folded GEP/global addressing,
    /// F128/x87) returns false and keeps the mature text path.
    /// Exact admission predicate for the typed i128 Load/Store route.
    ///
    /// The subset the isel arms accept, mirrored here so the generic
    /// operand scan never sees these instructions: an i128 value is
    /// slot-homed (the allocator never assigns one), so the generic
    /// no-register rules reject the Load dest / Store val before the isel
    /// could offer its typed forms. The predicate is the admission
    /// contract; the isel arm re-checks everything (belt and suspenders —
    /// a decline here falls back to the mature path unchanged).
    ///
    /// Subset: `ty.is_128bit()` (I128/U128 — F128/x87 long double stays on
    /// the mature x87 path), non-volatile (the volatile access granularity
    /// stays the oracle-matching two 8-byte halves), default address space,
    /// pointer = direct alloca slot or GPR-homed pointer value (never a
    /// folded address or an over-aligned alloca — the raw slot would be
    /// the wrong address), and the value side either an I128/Zero constant
    /// or a slot-homed non-small i128 value.
    fn i128_typed_candidate(
        &self,
        inst: &crate::ir::reexports::Instruction,
        folded_global_addrs: &crate::common::fx_hash::FxHashSet<u32>,
    ) -> bool {
        let (ty, ptr, val, volatile, seg_override) = match inst {
            crate::ir::reexports::Instruction::Load {
                ptr,
                ty,
                seg_override,
                volatile,
                ..
            } => (ty, ptr, None, *volatile, seg_override),
            crate::ir::reexports::Instruction::Store {
                val,
                ptr,
                ty,
                seg_override,
                volatile,
            } => (ty, ptr, Some(val), *volatile, seg_override),
            _ => return false,
        };
        if !ty.is_128bit() || volatile || *seg_override != AddressSpace::Default {
            return false;
        }
        let ptr_ok = (self.state.is_alloca(ptr.0)
            && !matches!(
                self.state.resolve_slot_addr(ptr.0),
                Some(crate::backend::state::SlotAddr::OverAligned(_, _))
            ))
            || self.reg_assignments.get(&ptr.0).is_some_and(|r| r.0 < 20);
        if !ptr_ok
            || folded_global_addrs.contains(&ptr.0)
            || self.state.folded_gep_values.contains(&ptr.0)
        {
            return false;
        }
        match val {
            None => {
                let dest = match inst {
                    crate::ir::reexports::Instruction::Load { dest, .. } => dest.0,
                    _ => unreachable!(),
                };
                // Genuine-i128-carrier requirement only (mirrors the
                // Store val's first check). Coherence does NOT apply to a
                // load dest: the load itself is the dest's defining write,
                // so the slot cannot be stale. Vector carriers (which also
                // ride the I128 type) are safe here too — writing the
                // loaded 16 bytes to the vector's spill slot eagerly is
                // data-identical to the vec machinery's own lazy spill.
                self.state.is_i128_value(dest)
                    && !self.reg_assignments.contains_key(&dest)
                    && self.state.get_slot(dest).is_some()
                    && !self.state.small_slot_values.contains(&dest)
            }
            Some(operand) => match operand {
                crate::ir::reexports::Operand::Const(c) => matches!(
                    c,
                    crate::ir::reexports::IrConst::I128(_) | crate::ir::reexports::IrConst::Zero
                ),
                crate::ir::reexports::Operand::Value(v) => {
                    // A GENUINE i128 carrier, not a 16-byte value that
                    // merely carries the I128 type in the IR: the vector
                    // path reuses I128 as the v4sf/v4si carrier type, and
                    // a vector value's spill slot is lazily written by the
                    // vec machinery (flush_pending_vec_store) — reading it
                    // here would load stale bytes.  The state's i128
                    // registry is the type truth; the builtin_ia32_
                    // vector_value regression pins the distinction.
                    // ...and its slot home must be COHERENT: written
                    // eagerly at the value's definition. A value computed
                    // into the accumulator pair has a stale slot (its only
                    // writer would be the very store being lowered).
                    self.state.is_i128_value(v.0)
                        && self.coherent_i128_homes.contains(&v.0)
                        && !self.reg_assignments.contains_key(&v.0)
                        && self.state.get_slot(v.0).is_some()
                        && !self.state.small_slot_values.contains(&v.0)
                }
            },
        }
    }

    fn float_typed_candidate(
        &self,
        inst: &crate::ir::reexports::Instruction,
        folded_global_addrs: &crate::common::fx_hash::FxHashSet<u32>,
    ) -> bool {
        use crate::common::types::{AddressSpace, IrType};
        use crate::ir::reexports::{Instruction, Operand};

        if self.machinst_disabled_kinds & MI_FLOAT_MOV != 0 {
            return false;
        }
        let ra = &self.reg_assignments;
        // Same value-domain exclusions as is_mi_unsafe_value minus the float
        // type check: accumulator locations, i128 carriers, and vectorizer
        // values may all be xmm-homed and F32/F64-typed in the IR while the
        // mature path treats them as packed/wide -- a scalar movsd would
        // truncate or stale their upper lanes.
        let xmm_homed = |v: u32| {
            ra.get(&v).is_some_and(|r| r.0 >= 20 && r.0 <= 33)
                && !self.state.is_accumulator_location(v)
                && !self.state.is_i128_value(v)
                && !self.state.vector_values.contains(&v)
        };
        // Spill-slot-homed float value: relayed through the pre-colored xmm0
        // scratch (PhysReg 18 — the allocator pool starts at xmm2, so the
        // scratch can never collide with an allocated home). Same value-domain
        // exclusions as xmm_homed: a vectorizer/i128/accumulator value that
        // happens to be slot-homed is packed/wide in the mature path's eyes
        // and must not be moved by a scalar relay.
        let float_slot_homed = |v: u32| {
            ra.get(&v).is_none()
                && self.state.get_slot(v).is_some()
                && !self.state.is_accumulator_location(v)
                && !self.state.is_i128_value(v)
                && !self.state.vector_values.contains(&v)
        };
        // Pointer admission mirrors the integer Load/Store ptr gate exactly:
        // folded GEP/global addresses have NO materialized register (the
        // producer was skipped -- reading its home is a segfault), and
        // over-aligned alloca slots need the aligned address the default
        // path computes.
        let gpr_ptr = |v: u32| {
            if folded_global_addrs.contains(&v) || self.state.folded_gep_values.contains(&v) {
                return false;
            }
            if let Some(crate::backend::state::SlotAddr::OverAligned(_, _)) =
                self.state.resolve_slot_addr(v)
            {
                return false;
            }
            self.state.is_alloca(v) || ra.get(&v).is_some_and(|r| r.0 < 20)
        };
        match inst {
            Instruction::Store {
                val,
                ptr,
                ty,
                seg_override,
                ..
            } if matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64) => {
                *seg_override == AddressSpace::Default
                    && matches!(val, Operand::Value(v) if xmm_homed(v.0) || float_slot_homed(v.0))
                    && gpr_ptr(ptr.0)
            }
            Instruction::Load {
                dest,
                ptr,
                ty,
                seg_override,
                ..
            } if matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64) => {
                *seg_override == AddressSpace::Default
                    && (xmm_homed(dest.0) || float_slot_homed(dest.0))
                    && gpr_ptr(ptr.0)
            }
            Instruction::Copy { dest, src } => {
                let Operand::Value(src_v) = src else {
                    return false;
                };
                let ty_is_scalar_float = self
                    .value_types
                    .get(&src_v.0)
                    .or_else(|| self.value_types.get(&dest.0))
                    .is_some_and(|ty| {
                        matches!(ty, IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64)
                    });
                ty_is_scalar_float && xmm_homed(src_v.0) && xmm_homed(dest.0)
            }
            Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } if self.machinst_disabled_kinds & MI_FLOAT_ALU == 0
                && matches!(ty, IrType::F32 | IrType::F64)
                && matches!(
                    op,
                    crate::ir::reexports::IrBinOp::Add
                        | crate::ir::reexports::IrBinOp::Sub
                        | crate::ir::reexports::IrBinOp::Mul
                        | crate::ir::reexports::IrBinOp::SDiv
                ) =>
            {
                // Dest must be xmm-homed; then either lhs is xmm-homed (rhs
                // xmm or slot), or -- commutative ops only -- rhs is
                // xmm-homed and lhs slot-homed. Mirrors the isel arm.
                let (Operand::Value(l), Operand::Value(r)) = (lhs, rhs) else {
                    return false;
                };
                if !xmm_homed(dest.0) {
                    return false;
                }
                let has_slot = |v: u32| self.state.get_slot(v).is_some();
                if xmm_homed(l.0) {
                    return xmm_homed(r.0) || has_slot(r.0);
                }
                let commutative = matches!(
                    op,
                    crate::ir::reexports::IrBinOp::Add | crate::ir::reexports::IrBinOp::Mul
                );
                commutative && xmm_homed(r.0) && has_slot(l.0)
            }
            _ => false,
        }
    }
    /// Typed direct-call lowering: the SysV integer-register subset as one
    /// atomic [`MachInst::CallTyped`].
    ///
    /// Admission contract (everything else keeps the mature path):
    /// - direct call to a named, non-empty symbol; not variadic, not sret;
    /// - no struct/i128/F128/x87 arguments; at most six integer/pointer
    ///   arguments of ABI width 4 or 8 (narrow C arguments are moved at
    ///   their canonical 32-bit promotion width, per the same convention
    ///   the load path's `narrow_defined_load` establishes);
    /// - integer/pointer return (or void) whose destination home is a GPR
    ///   or a spill slot; dead/accumulator/xmm destinations stay behind;
    /// - inline-memcpy candidates keep their interception in the dispatch
    ///   loop (an inlined fixed-size memcpy beats any call sequence);
    /// - the pure builder must produce an acyclic move plan.
    ///
    /// Flushing discipline: the preceding run is drained first, the call is
    /// buffered as its own run, and that run is flushed immediately. This
    /// reproduces the mature path's program-point semantics exactly — the
    /// caller-save interval check runs at the call's own point — and keeps
    /// the pre-colored argument/return registers conflict-free, since no
    /// allocated vreg can coexist with the call in the buffer. The flush's
    /// tail (invalidate_all + pending-vec-store flush + vec-peephole
    /// invalidation) is precisely `clobber_after_call_like`.
    fn try_lower_call_typed(
        &mut self,
        inst: &crate::ir::reexports::Instruction,
        direct_sym: Option<&str>,
        func_ptr: Option<&crate::ir::reexports::Operand>,
        info: &crate::ir::reexports::CallInfo,
        folded_global_addrs: &crate::common::fx_hash::FxHashSet<u32>,
    ) -> bool {
        use super::isel::{build_typed_call, build_typed_call_ex, TypedCallSrc};
        use super::machinst::{CallArgMove, CallTarget, MachInst, MachOperand, MachReg, OpSize};

        let _ = folded_global_addrs; // subset needs no fold-map interaction

        if self.machinst_disabled_kinds & MI_CALL_TYPED != 0 {
            return false;
        }
        // Inline fixed-size memcpy/__memcpy_chk must keep winning over any
        // call sequence (dispatch-loop interception, mirrored gate).
        if crate::backend::generation::inline_memcpy_len(
            direct_sym.unwrap_or(""),
            &info.args,
            info.is_variadic,
        )
        .is_some()
        {
            return false;
        }
        // Non-variadic, no sret. Direct calls need a named symbol; indirect
        // calls are admitted through the callee-pointer resolution below.
        if info.is_variadic || info.is_sret {
            return false;
        }
        if direct_sym.is_none() && func_ptr.is_none() {
            return false;
        }
        // No struct/by-value aggregates (the classification would leave the
        // integer-register subset).
        if info.struct_arg_sizes.iter().any(|s| s.is_some()) {
            return false;
        }
        if info.args.len() > super::machinst::SYSCALL_ARG_REGS.len() {
            return false;
        }
        // Integer/pointer domain for every argument and the return.
        let width_ok = |ty: IrType| {
            !ty.is_128bit() && !ty.is_float() && !ty.is_long_double() && ty.size() <= 8
        };
        if !width_ok(info.return_type) || info.arg_types.iter().any(|t| !width_ok(*t)) {
            return false;
        }

        // Canonical move width: never narrower than 32 bits. Narrow C
        // arguments are passed at their promoted width (the ABI reads the
        // 32-bit register; a byte/word move would leave the upper bits of
        // the argument register undefined).
        let move_width = |ty: IrType| match OpSize::from_ir_type(ty) {
            OpSize::S8 | OpSize::S16 => OpSize::S32,
            s => s,
        };

        let ra = &self.reg_assignments;
        let state = &self.state;
        let value_types = &self.value_types;

        // Resolve an argument operand to a typed source. None = the mature
        // path stages it (accumulator/xmm/alloca-address/rbp homes, dead
        // values, non-integer constants).
        let arg_src = |op: &crate::ir::reexports::Operand| -> Option<TypedCallSrc> {
            match op {
                crate::ir::reexports::Operand::Const(c) => c.to_i64().map(TypedCallSrc::Imm),
                crate::ir::reexports::Operand::Value(v) => {
                    // Address-of-local arguments (`&x`): a separate pre-move
                    // (Mov { AllocaAddr } → leaq by the resolver) places the
                    // address into the ABI register before the call. The
                    // slot must exist for the resolver to render the leaq.
                    if state.is_alloca(v.0) {
                        return state.get_slot(v.0).map(|_| TypedCallSrc::AllocaAddr(v.0));
                    }
                    if is_mi_unsafe_value(v.0, ra, state, value_types) {
                        return None;
                    }
                    if let Some(&r) = ra.get(&v.0) {
                        if r.0 >= 20 || r.0 == 0 || r.0 == 6 || r.0 == 7 {
                            return None; // xmm / rax / rbp / rcx homes
                        }
                        return Some(TypedCallSrc::Reg(r));
                    }
                    state.get_slot(v.0).map(|slot| TypedCallSrc::Slot(slot.0))
                }
            }
        };

        let arg_srcs: Vec<Option<TypedCallSrc>> = info.args.iter().map(arg_src).collect();
        let arg_sizes: Vec<OpSize> = info.arg_types.iter().map(|t| move_width(*t)).collect();

        // Return home (None = void call; a dead destination defers to the
        // mature path rather than dropping the store).
        let ret_plan = match &info.dest {
            None => None,
            Some(d) => {
                if state.is_alloca(d.0) || is_mi_unsafe_value(d.0, ra, state, value_types) {
                    return false;
                }
                let src = if let Some(&r) = ra.get(&d.0) {
                    if r.0 >= 20 || r.0 == 0 || r.0 == 6 {
                        return false;
                    }
                    TypedCallSrc::Reg(r)
                } else if let Some(slot) = state.get_slot(d.0) {
                    TypedCallSrc::Slot(slot.0)
                } else {
                    return false;
                };
                Some((Some(src), move_width(info.return_type)))
            }
        };

        // Decide the call form and build the move plan together.
        //
        // Direct target: symbol naming mirrors emit_call_instruction_impl —
        // versioned symbols use the base name in @PLT references (GAS 2.47
        // rejects `sym@ver@PLT`; the linker resolves the version).
        //
        // Indirect target: the callee pointer is staged into a scratch
        // register by one of the instruction's own moves (see
        // build_typed_call_ex). Candidate registers are r10 then r11 —
        // never ABI argument registers, so no argument move can write
        // them. Under an EXTERNAL retpoline thunk the call form is
        // `call __x86_indirect_thunk_r10`, whose name encodes the register
        // and whose patching contract the mature path owns: reject the
        // typed path for indirect calls in both thunk modes instead of
        // approximating it. The paravirt fast-path (callee resolves to
        // global+offset) must likewise keep the mature path: it emits the
        // patchable `call *sym+off(%rip)` the kernel alternatives
        // machinery rewrites at boot.
        let (plan, target): (super::isel::TypedCallPlan, CallTarget) = if let Some(sym) = direct_sym
        {
            let target = if self.state.needs_plt(sym) {
                let n = sym.split('@').next().unwrap_or(sym);
                CallTarget::Direct(format!("{}@PLT", n))
            } else {
                CallTarget::Direct(sym.to_string())
            };
            match build_typed_call(&arg_srcs, &arg_sizes, ret_plan) {
                Ok(p) => (p, target),
                Err(reason) => {
                    super::isel::stats::note_reject_named(reason.label());
                    return false;
                }
            }
        } else {
            let fp = func_ptr.expect("indirect call without func_ptr");
            if self.state.indirect_branch_thunk || self.state.indirect_branch_thunk_inline {
                return false;
            }
            if self.try_resolve_indirect_call_global(fp).is_some() {
                return false; // patchable `call *sym(%rip)` form is load-bearing
            }
            let callee_src = match arg_src(fp) {
                Some(s) => s,
                None => {
                    // Accumulator-homed callees are the dominant shape here
                    // (a GOT load with no allocator home leaves the pointer
                    // in %rax and the mature path relays it into r10).
                    // Visible rejection, not a silent fallback: the census
                    // must rank this or the next round cannot target it.
                    super::isel::stats::note_reject_named("CallIndirect(callee-unrepresentable)");
                    return false;
                }
            };
            let mut built: Option<(super::isel::TypedCallPlan, CallTarget)> = None;
            for candidate in [super::machinst::R10, super::machinst::R11] {
                if let Ok(p) = build_typed_call_ex(
                    &arg_srcs,
                    &arg_sizes,
                    ret_plan,
                    Some((callee_src, candidate)),
                ) {
                    built = Some((p, CallTarget::Indirect(candidate)));
                    break;
                }
            }
            match built {
                Some(pair) => pair,
                None => {
                    // Both scratch candidates dead (e.g. a register-exchange
                    // cycle involving r10 and r11): the text path owns the
                    // hazard-spill machinery for that shape.
                    super::isel::stats::note_reject_named("CallIndirect(no-scratch)");
                    return false;
                }
            }
        };

        // Caller-save spills at the call's program point — the same data
        // and the same point semantics as the mature path's Phase 2b.
        let point = self.state.current_program_point;
        let mut caller_saves: Vec<(PhysReg, i64)> = Vec::new();
        for (&reg_id, &slot) in &self.caller_save_spill_slots {
            if let Some(intervals) = self.caller_save_intervals.get(&reg_id) {
                if intervals
                    .iter()
                    .any(|&(start, end)| start <= point && point <= end)
                {
                    caller_saves.push((PhysReg(reg_id), slot.0));
                }
            }
        }

        // Split alloca-address arguments into pre-moves. The pre-move
        // writes the ABI register BEFORE the caller-save spills run, so a
        // register in the save set would be spilled after being clobbered —
        // reject that shape (the mature path orders saves before staging).
        // The builder already guaranteed no other argument sources from the
        // register, so hoisting all pre-moves to the front of the run is
        // order-equivalent.
        let mut pre_moves: Vec<MachInst> = Vec::new();
        let mut typed_args: Vec<CallArgMove> = Vec::new();
        for m in plan.args {
            match &m.src {
                MachOperand::AllocaAddr(id) => {
                    if caller_saves.iter().any(|(r, _)| *r == m.dst_reg) {
                        return false;
                    }
                    pre_moves.push(MachInst::Mov {
                        src: MachOperand::AllocaAddr(*id),
                        dst: MachOperand::Reg(MachReg::Phys(m.dst_reg)),
                        size: OpSize::S64,
                    });
                }
                _ => typed_args.push(m),
            }
        }

        // Drain the preceding run, buffer the call as its own run, flush it.
        self.flush_machinst();
        self.machinst_buf.extend(pre_moves);
        self.machinst_buf.push(MachInst::CallTyped {
            caller_saves,
            args: typed_args,
            target,
            ret: plan.ret,
        });
        self.machinst_buf_ir.push(inst.clone());
        self.flush_machinst();
        super::isel::stats::note_lowered();
        true
    }

    // ── Typed ParamRef materialization ───────────────────────────────
    //
    // The census's `ParamRef(needs-code)` class: parameters whose body-side
    // materialization the text path performs at the price of flushing the
    // buffered MachInst run around them. Three of the four
    // `emit_param_ref_impl` cases are single moves, which MachInst expresses
    // exactly — the admission below mirrors the text path instruction for
    // instruction, so the final machine state (destination register or slot
    // contents) is byte-identical in every admitted shape while the run
    // stays unsplit.

    /// The single-instruction load form that materializes a value of type
    /// `load_ty` into a 64-bit GPR with a final register state byte-identical
    /// to the text path's `mov_load_for_type(load_ty)` + `store_rax_to` pair.
    ///
    /// The text path loads into %rax at the mnemonic's width and then
    /// `movq`s the FULL 64-bit register into the destination (see
    /// `store_rax_to`: "movl zero-extends the upper 32 bits, which corrupts
    /// negative I32 values"), so the admitted forms must reproduce the same
    /// 64-bit destination state in one instruction:
    ///   * signed narrow loads sign-extend all the way (`movsbq/movswq/movslq`);
    ///   * unsigned 8/16-bit loads zero-extend (`movzbl/movzwl` — the
    ///     MachInst emitter renders them with a 32-bit dest name, which
    ///     zero-extends into the full register implicitly);
    ///   * U32 loads are plain `movl` (zero-extends into the full register);
    ///   * 64-bit loads are plain `movq`.
    /// `None` = no admitted form (float/decimal/F128/wide — those stay on
    /// the text path, which owns the SSE and x87 domains).
    fn paramref_reg_load_form(load_ty: IrType) -> Option<ParamRefLoad> {
        Some(match load_ty {
            IrType::U8 => ParamRefLoad::Zext(OpSize::S8),
            IrType::U16 => ParamRefLoad::Zext(OpSize::S16),
            IrType::I8 => ParamRefLoad::Sext(OpSize::S8),
            IrType::I16 => ParamRefLoad::Sext(OpSize::S16),
            IrType::I32 => ParamRefLoad::Sext(OpSize::S32),
            IrType::U32 => ParamRefLoad::Raw32,
            IrType::I64 | IrType::U64 | IrType::Ptr => ParamRefLoad::Raw64,
            _ => return None,
        })
    }

    /// Push the extending/plain load `mov {src} → {MachReg}` for a form.
    fn push_paramref_reg_load(&mut self, form: ParamRefLoad, src: MachOperand, dst: MachReg) {
        let inst = match form {
            ParamRefLoad::Zext(from) => MachInst::Movzx {
                src,
                dst,
                from_size: from,
                to_size: OpSize::S64,
            },
            ParamRefLoad::Sext(from) => MachInst::Movsx {
                src,
                dst,
                from_size: from,
                to_size: OpSize::S64,
            },
            ParamRefLoad::Raw32 => MachInst::Mov {
                src,
                dst: MachOperand::Reg(dst),
                size: OpSize::S32,
            },
            ParamRefLoad::Raw64 => MachInst::Mov {
                src,
                dst: MachOperand::Reg(dst),
                size: OpSize::S64,
            },
        };
        self.machinst_buf.push(inst);
    }

    /// Admit a slot-homed destination for a slot-sourced load: ONLY the
    /// shapes whose stored bytes are byte-identical to the text path's
    /// relay (`mov_load_for_type` into %rax, then `store_rax_to`).
    ///
    /// The emitter lowers a Mov with two memory operands as an %rax relay at
    /// the instruction's width, so the admitted widths are exactly the ones
    /// where the text relay's width equals the MachInst relay's width:
    ///   * 64-bit types into an 8-byte slot: `movq`/`movq` both ways;
    ///   * I32/U32 into a 4-byte (small) slot: the text path stores the low
    ///     4 bytes either way (`movl %eax, slot` after movslq/movl staging)
    ///     — the relay's raw 4-byte copy writes the identical bytes.
    /// Everything else (narrow types whose relay would store fewer bytes
    /// than the text path's full-width store, I32 into an 8-byte slot whose
    /// sign extension the text path materializes) stays on the text path.
    fn paramref_slot_relay_ok(load_ty: IrType, small_dest: bool) -> Option<OpSize> {
        match load_ty {
            IrType::I64 | IrType::U64 | IrType::Ptr if !small_dest => Some(OpSize::S64),
            IrType::I32 | IrType::U32 if small_dest => Some(OpSize::S32),
            _ => None,
        }
    }

    /// Lower one ParamRef materialization into the MachInst buffer.
    ///
    /// `Ok(())` = lowered (possibly zero instructions: dead destination,
    /// pre-stored parameter, already-in-place register, or a class the text
    /// path emits nothing for). `Err(label)` = reject; the label feeds the
    /// census so the residual classes stay individually counted.
    ///
    /// Admission mirrors `emit_param_ref_impl` (prologue.rs) case for case:
    /// pre-stored parameters win over the alloca slot; the alloca slot wins
    /// over the ABI class; the pinned `# LCCC_PARAM_ABI_READ` marker
    /// accompanies every incoming-register read (the text peephole's
    /// `pin_param_abi_reads` runs on the final text, where MachInst output
    /// is already flushed, so the marker reaches it unchanged).
    fn try_lower_paramref_typed(
        &mut self,
        dest: &Value,
        param_idx: usize,
        ty: IrType,
    ) -> Result<(), &'static str> {
        use super::machinst::{MachOperand, MachReg, SYSCALL_ARG_REGS};

        // A dead destination gets no code on the text path either
        // (`store_rax_to` early-outs on zero use counts); lowering it as a
        // no-op keeps the run unsplit at zero cost.
        if self
            .state
            .value_use_counts
            .get(dest.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0
        {
            return Ok(());
        }
        // Pre-stored parameters are already in their destination register;
        // `emit_store_params` placed them during the prologue. This takes
        // priority over the alloca slot exactly as on the text path.
        if self.state.param_pre_stored.contains(&param_idx) {
            return Ok(());
        }
        // Umbrella home-safety guard — the same one the typed calls use:
        // accumulator/i128/vector/xmm-homed destinations and float/decimal/
        // F128/wide destination types keep the mature path.
        if is_mi_unsafe_value(
            dest.0,
            &self.reg_assignments,
            &self.state,
            &self.value_types,
        ) {
            return Err("ParamRef(unsafe-home)");
        }
        // Resolve the destination home.
        let dst = if let Some(&r) = self.reg_assignments.get(&dest.0) {
            if r.0 >= 20 || r.0 == 0 || r.0 == 6 || r.0 == 7 {
                return Err("ParamRef(unsafe-home)"); // xmm / rax / rbp / rcx
            }
            ParamDst::Reg(r)
        } else if let Some(slot) = self.state.get_slot(dest.0) {
            ParamDst::Slot(slot.0)
        } else {
            return Err("ParamRef(unsafe-home)"); // no home: text path owns
        };
        let small_dest = self.state.is_small_slot(dest.0);

        // Case 1 (text path's first materializing case): the parameter's
        // alloca slot holds the staged value — one load.
        if param_idx < self.state.param_alloca_slots.len() {
            if let Some((slot, alloca_ty)) = self.state.param_alloca_slots[param_idx] {
                return match dst {
                    ParamDst::Reg(r) => {
                        let form = Self::paramref_reg_load_form(alloca_ty)
                            .ok_or("ParamRef(float-domain)")?;
                        self.push_paramref_reg_load(
                            form,
                            MachOperand::StackSlot(slot.0),
                            MachReg::Phys(r),
                        );
                        Ok(())
                    }
                    ParamDst::Slot(dslot) => {
                        let size = Self::paramref_slot_relay_ok(alloca_ty, small_dest)
                            .ok_or("ParamRef(slot-width)")?;
                        self.machinst_buf.push(MachInst::Mov {
                            src: MachOperand::StackSlot(slot.0),
                            dst: MachOperand::StackSlot(dslot),
                            size,
                        });
                        Ok(())
                    }
                };
            }
        }
        if param_idx >= self.state.param_classes.len() {
            // Unreachable through the gate (which no-codes out-of-bounds
            // indices first), but the match below indexes the vector.
            return Err("ParamRef(unsafe-home)");
        }
        let class = self.state.param_classes[param_idx];
        match class {
            // Case 2: the parameter arrived in an ABI register and must be
            // copied to its (different) home. Every emitted read carries the
            // pinned-ABI-read marker; the pin pass on the final text pins
            // the very next instruction line, which is exactly the move
            // pushed here (single-line forms only: reg→reg and reg→slot
            // render as one line each).
            ParamClass::IntReg { reg_idx } => {
                if reg_idx >= SYSCALL_ARG_REGS.len() {
                    return Err("ParamRef(unsafe-home)");
                }
                let abi = SYSCALL_ARG_REGS[reg_idx];
                if let ParamDst::Reg(r) = dst {
                    if r == abi {
                        // The destination IS the incoming register: the
                        // value is already in place. Only a 64-bit type is
                        // truly in place — a narrower type needs the
                        // extending move, and the emitter's size-blind
                        // self-move elision would drop it, so those stay on
                        // the text path (whose round trip materializes the
                        // extension).
                        if OpSize::from_ir_type(ty) == OpSize::S64 {
                            return Ok(());
                        }
                        return Err("ParamRef(narrow-self)");
                    }
                }
                // The marker names the sized incoming register, spelled
                // exactly as the text path spells it (`reg_for_type`).
                let src_name = Self::reg_for_type(X86_ARG_REGS[reg_idx], ty);
                self.machinst_buf.push(MachInst::Raw(format!(
                    "    # LCCC_PARAM_ABI_READ {}",
                    src_name
                )));
                match dst {
                    ParamDst::Reg(r) => {
                        let form =
                            Self::paramref_reg_load_form(ty).ok_or("ParamRef(float-domain)")?;
                        self.push_paramref_reg_load(
                            form,
                            MachOperand::Reg(MachReg::Phys(abi)),
                            MachReg::Phys(r),
                        );
                    }
                    ParamDst::Slot(dslot) => {
                        // reg→slot is a single encodable store; admit the
                        // same byte-identical widths as the relay case.
                        let size = Self::paramref_slot_relay_ok(ty, small_dest)
                            .ok_or("ParamRef(slot-width)")?;
                        self.machinst_buf.push(MachInst::Mov {
                            src: MachOperand::Reg(MachReg::Phys(abi)),
                            dst: MachOperand::StackSlot(dslot),
                            size,
                        });
                    }
                }
                Ok(())
            }
            // Case 3: stack-passed scalar. The incoming-argument area is
            // addressed through the same slot-offset formula the text path
            // uses (`slot_ref(stack_base + offset)`), and the MachOperand::
            // StackSlot formatter applies the identical rbp/rsp-mode rule —
            // the rendered operand is byte-identical.
            ParamClass::StackScalar { offset } => {
                let stack_base: i64 = if self.state.omit_frame_pointer { 8 } else { 16 };
                let src_off = stack_base + offset;
                match dst {
                    ParamDst::Reg(r) => {
                        let form =
                            Self::paramref_reg_load_form(ty).ok_or("ParamRef(float-domain)")?;
                        self.push_paramref_reg_load(
                            form,
                            MachOperand::StackSlot(src_off),
                            MachReg::Phys(r),
                        );
                        Ok(())
                    }
                    ParamDst::Slot(dslot) => {
                        let size = Self::paramref_slot_relay_ok(ty, small_dest)
                            .ok_or("ParamRef(slot-width)")?;
                        self.machinst_buf.push(MachInst::Mov {
                            src: MachOperand::StackSlot(src_off),
                            dst: MachOperand::StackSlot(dslot),
                            size,
                        });
                        Ok(())
                    }
                }
            }
            // GPR↔XMM relays stay on the text path by design (the MachInst
            // float layer is xmm-domain only).
            ParamClass::FloatReg { .. } => Err("ParamRef(float-domain)"),
            // i128 pairs, by-value structs, F128, stack structs, …: the
            // text path's default arm emits NOTHING for these (their
            // prologue staging is the whole story), so a no-op lowering is
            // semantically identical — and keeps the run unsplit.
            _ => Ok(()),
        }
    }
}

impl ArchCodegen for X86Codegen {
    fn is_value_reg_assigned(&self, vid: u32) -> bool {
        self.reg_assignments.contains_key(&vid)
    }

    fn flush_pending_widen(&mut self) {
        self.flush_pending_widen_impl();
    }

    fn narrow_cmp_operands(
        &mut self,
        dest: u32,
        op: crate::ir::reexports::IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) -> Option<(Operand, Operand, IrType, crate::ir::reexports::IrCmpOp)> {
        self.narrow_cmp_operands_impl(dest, op, lhs, rhs)
    }

    fn state(&mut self) -> &mut CodegenState {
        &mut self.state
    }
    fn state_ref(&self) -> &CodegenState {
        &self.state
    }
    fn ptr_directive(&self) -> PtrDirective {
        PtrDirective::Quad
    }

    fn supports_mcount(&self) -> bool {
        true
    }

    fn supports_classic_mcount(&self) -> bool {
        true
    }

    fn get_phys_reg_for_value(&self, val_id: u32) -> Option<PhysReg> {
        self.reg_assignments.get(&val_id).copied()
    }

    fn is_machinst_enabled(&self) -> bool {
        self.machinst_enabled && self.machinst_function_enabled
    }

    fn supports_inline_memcpy_call(&self) -> bool {
        true
    }

    fn fp_contract(&self) -> crate::common::fp_contract::FpContract {
        self.fp_contract
    }

    fn supports_fused_float_mul_add(&self) -> bool {
        // FP contraction changes results (single rounding). Only fuse under
        // the explicit contract; integer mul+add fusion is unaffected (the
        // generation-side detector calls this only for float types).
        // FMA3 is additionally gated on the target ISA: vfmadd231s{s,d} is
        // illegal on baseline (SSE2) x86-64, matching GCC, which requires
        // `-mfma` (or an enabling -march) even under -ffp-contract=fast.
        self.fp_contract != FpContract::Off && self.fma_enabled
    }

    fn supports_fused_float_mul_sub(&self) -> bool {
        // vfmsub231/vfnmadd231 need the same contraction contract *and* the
        // same FMA3 ISA gate as vfmadd231.
        self.fp_contract != FpContract::Off && self.fma_enabled
    }

    fn emit_fused_mul_sub(
        &mut self,
        _mul_dest: &Value,
        mul_lhs: &Operand,
        mul_rhs: &Operand,
        acc: &Operand,
        sub_dest: &Value,
        ty: IrType,
        mul_is_lhs: bool,
    ) {
        debug_assert!(matches!(ty, IrType::F32 | IrType::F64));
        self.emit_scalar_fms231(mul_lhs, mul_rhs, acc, sub_dest, ty, mul_is_lhs);
    }

    fn supports_and_not(&self) -> bool {
        self.bmi1_enabled && std::env::var_os("CCC_NO_ANDN_FUSION").is_none()
    }

    fn supports_fused_bit_test_branch(&self) -> bool {
        // BT + JC consumes CF directly: no setc/movzx/test materialization.
        // Kill switch for bisection parity with the other branch fusions.
        std::env::var_os("CCC_NO_BT_BRANCH_FUSION").is_none()
    }

    fn emit_and_not(
        &mut self,
        _not_dest: &Value,
        not_src: &Operand,
        other: &Operand,
        dest: &Value,
        ty: IrType,
        direct_return: bool,
    ) {
        self.emit_and_not_impl(not_src, other, dest, ty, direct_return);
    }

    fn try_lower_machinst(
        &mut self,
        inst: &crate::ir::reexports::Instruction,
        folded_global_addrs: &crate::common::fx_hash::FxHashSet<u32>,
    ) -> bool {
        if !self.machinst_enabled
            || !self.machinst_function_enabled
            || self.machinst_disabled_kinds & machinst_kind_bit(inst) != 0
        {
            return false;
        }
        // ParamRef: lower the no-code subset AND the single-move
        // materializations through MachInst.
        //
        // A ParamRef is not a no-op in general -- `emit_param_ref_impl` has
        // four cases: pre-stored (nothing to do), a parameter homed in an
        // alloca slot (a load), an incoming ABI register that must be copied
        // to a different home, and a stack-passed argument loaded from
        // `16(%rbp)`/`8(%rsp)`. A previous attempt returned `true` for all of
        // them and broke four corpus tests, every one parameter-related --
        // those cases genuinely materialize, so the typed path below lowers
        // them as moves (mirroring the text path instruction for
        // instruction) instead of no-opping them.
        //
        // The pre-stored subset is different in kind: `emit_store_params`
        // already placed the value in the destination's register during the
        // prologue, so the body emits literally nothing. Buffering it costs
        // nothing and, crucially, does NOT flush the MachInst run -- which is
        // the whole point, because a ParamRef sitting between two lowerable
        // instructions used to split them onto separate paths. Parameters are
        // 10.8% of all instructions in the corpus.
        //
        // Whatever the typed path refuses keeps the text path, where the ABI
        // contract (including the pinned "read the *incoming* register even
        // if another pre-store aliased that name" rule) is already
        // implemented; the typed path emits that same marker for its own
        // incoming-register reads.
        if let crate::ir::reexports::Instruction::ParamRef {
            dest,
            param_idx,
            ty,
        } = inst
        {
            let no_code = *param_idx >= self.state.param_classes.len()
                || (self.state.param_pre_stored.contains(param_idx)
                    && self
                        .state
                        .param_alloca_slots
                        .get(*param_idx)
                        .copied()
                        .flatten()
                        .is_none());
            if no_code {
                super::isel::stats::note_lowered();
                return true;
            }
            match self.try_lower_paramref_typed(dest, *param_idx, *ty) {
                Ok(()) => {
                    super::isel::stats::note_lowered();
                    return true;
                }
                Err(label) => {
                    super::isel::stats::note_reject_named(label);
                    return false;
                }
            }
        }

        // Typed calls (CallTyped): the SysV integer-register subset — direct
        // AND indirect — lowered as one atomic machine instruction. Indirect
        // calls stage the callee pointer into r10/r11 inside the same move
        // list. Everything else — variadic/sret/struct/i128/F128 calls,
        // stack-overflow arguments, register-exchange hazard cycles, inline-
        // memcpy candidates, retpoline/paravirt indirect forms — keeps the
        // mature path, which owns the general contract. See
        // try_lower_call_typed for the full admission contract.
        if let crate::ir::reexports::Instruction::Call { func, info } = inst {
            return self.try_lower_call_typed(inst, Some(func), None, info, folded_global_addrs);
        }
        if let crate::ir::reexports::Instruction::CallIndirect { func_ptr, info } = inst {
            return self.try_lower_call_typed(
                inst,
                None,
                Some(func_ptr),
                info,
                folded_global_addrs,
            );
        }

        // Div/rem pair heads and tails must stay on the classic emitter path:
        // the fusion (emit_divrem_pair_head) lives there and the MachInst
        // lowering emits standalone divisions for each side, which would both
        // duplicate the divide and skip the tail elimination.
        if let crate::ir::reexports::Instruction::BinOp {
            dest,
            op:
                crate::ir::reexports::IrBinOp::SDiv
                | crate::ir::reexports::IrBinOp::UDiv
                | crate::ir::reexports::IrBinOp::SRem
                | crate::ir::reexports::IrBinOp::URem,
            ..
        } = inst
        {
            if self.divrem_tail_dests.contains(&dest.0)
                || self.divrem_head_partners.contains_key(&dest.0)
                || self.divrem_broken_tails.contains(&dest.0)
            {
                return false;
            }
        }
        // W2 Load->Cast folding is a two-instruction runtime handshake: the
        // default Load emitter redirects into the Cast destination and arms
        // fold_skip_cast; the default Cast emitter consumes that handshake.
        // Lowering the Cast through MachInst bypasses the consumer and emits a
        // second conversion from the load's stale original home. This corrupted
        // gzip's gzip_deflate/compress_block byte loads. Keep candidate Casts on
        // the default path; if a particular Load could not redirect, the default
        // Cast simply emits normally.
        if let crate::ir::reexports::Instruction::Cast {
            dest,
            from_ty,
            to_ty,
            ..
        } = inst
        {
            if self.folded_cast_dests.contains(&dest.0) {
                return false;
            }
            // Float casts (and F128) stay on the mature path. The MachInst
            // lower_cast only knows integer size-moves: a same-size
            // Ptr(8) -> F64 cast would be emitted as `movq %reg, %xmm0` — a
            // BITCAST of the address, not a cvtsi2sdq value conversion
            // (`(double)ptr` printed 0.0 for a 0x55… address). Mirrors
            // isel::can_lower, which already excludes floats; this gate used
            // a coarser size/sign classification and missed them.
            if from_ty.is_float() || to_ty.is_float() {
                return false;
            }
            // Signed widening remains on the mature path. Although the raw
            // movsx selection is straightforward, SQLite's optimized VDBE has
            // overlapping allocator locations where the mature cast emitter's
            // type-aware relay/liveness handling is required; MachInst corrupted
            // an UnpackedRecord pointer before sqlite3BtreeIndexMoveto. Category
            // bisection proved this exact class, while unsigned widening,
            // narrowing, and same-width casts pass. This guard is narrow and
            // preserves all measured-safe MachInst coverage.
            if to_ty.size() > from_ty.size() && !from_ty.is_unsigned() {
                return false;
            }
            let shape_bit = match to_ty.size().cmp(&from_ty.size()) {
                std::cmp::Ordering::Greater => MI_CAST_WIDEN,
                std::cmp::Ordering::Less => MI_CAST_NARROW,
                std::cmp::Ordering::Equal => MI_CAST_SAME,
            };
            let sign_bit = if from_ty.is_unsigned() {
                MI_CAST_UNSIGNED_SOURCE
            } else {
                MI_CAST_SIGNED_SOURCE
            };
            if self.machinst_disabled_kinds & (shape_bit | sign_bit) != 0 {
                return false;
            }
        }
        // Keep Select on the mature path. MachInst's current lowering reserves
        // rax for the true value, but resolving a stack-backed condition also
        // uses rax as a relay, clobbering that true value before cmov. The mature
        // path additionally consumes pending Cmp flags directly and is both
        // smaller and faster. Keep the preceding fused Cmp there as well so the
        // boolean is never needlessly materialized (gzip huft_build regression).
        match inst {
            crate::ir::reexports::Instruction::Select { .. } => return false,
            // PF-15: single-use widening casts from byte/half types stay on
            // the mature path, where try_record_pending_widen can defer them
            // into a narrow compare. MachInst would emit the extending move
            // eagerly, which forecloses the fold. Eligibility mirrors
            // try_record_pending_widen (the record path re-checks everything
            // and emits normally on any mismatch, so a lenient gate here is
            // merely an optimization hint, never a correctness input).
            crate::ir::reexports::Instruction::Cast {
                dest,
                from_ty,
                to_ty,
                ..
            } if matches!(from_ty, IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16)
                && !to_ty.is_float()
                && (to_ty.size() == 4 || to_ty.size() == 8)
                && self.value_use_counts.get(&dest.0).copied() == Some(1) =>
            {
                return false;
            }
            // PF-16b: widening casts of provably-nonnegative bitop results
            // reclassify to the unsigned path in the mature emitter; MachInst
            // would sign-extend eagerly (cltq/movslq) and foreclose that.
            crate::ir::reexports::Instruction::Cast {
                src: Operand::Value(sv),
                from_ty,
                to_ty,
                ..
            } if from_ty.is_signed()
                && from_ty.size() <= 4
                && to_ty.size() == 8
                && self.bitop_nonneg_values.contains(&sv.0) =>
            {
                return false;
            }
            crate::ir::reexports::Instruction::Cmp { dest, .. }
                if self.fused_cmp_dests.contains_key(&dest.0)
                    || self.cmp_replay.contains_key(&dest.0) =>
            {
                // Fused candidates keep their flags for the adjacent consumer;
                // REPLAY candidates are emitted by the mature path's Cmp
                // emitter, which skips the boolean materialization entirely
                // (the consumer re-emits the compare). Sending either through
                // MachInst would materialize setcc/movzbl AND leave the
                // consumer to replay — a doubled compare per select.
                return false;
            }
            _ => {}
        }
        // Debug knob: CCC_MI_SKIP_FUNC=name1,name2 disables the MachInst path
        // for whole functions (diagnostic bisection aid).
        if let Some(skip) = std::env::var("CCC_MI_SKIP_FUNC").ok() {
            if skip
                .split(',')
                .any(|n| n == self.state.current_func_name.as_str())
            {
                return false;
            }
        }

        // Scalar float FMov bypass. The generic operand scan below rejects
        // every XMM-domain value (is_mi_unsafe_value), so the validated
        // float subset routes straight to the typed lowering. The candidate
        // check is the exact admission predicate for the isel float arms;
        // if the lowering still declines (belt and suspenders), the text
        // path takes over unchanged.
        if self.float_typed_candidate(inst, folded_global_addrs) {
            let mut float_alloca_slots = crate::common::fx_hash::FxHashMap::default();
            // The float arms read the VALUE's spill slot (relay source) and
            // the DEST's spill slot (relay target) from the same map as the
            // pointer slot — insert all three shapes the instruction has.
            match inst {
                crate::ir::reexports::Instruction::Load { dest, ptr, .. } => {
                    for v in [ptr.0, dest.0] {
                        if self.state.is_alloca(v) {
                            if let Some(slot) = self.state.get_slot(v) {
                                float_alloca_slots.insert(v, slot.0);
                            }
                        }
                    }
                    if !self.reg_assignments.contains_key(&dest.0) {
                        if let Some(slot) = self.state.get_slot(dest.0) {
                            float_alloca_slots.insert(dest.0, slot.0);
                        }
                    }
                }
                crate::ir::reexports::Instruction::Store { val, ptr, .. } => {
                    if self.state.is_alloca(ptr.0) {
                        if let Some(slot) = self.state.get_slot(ptr.0) {
                            float_alloca_slots.insert(ptr.0, slot.0);
                        }
                    }
                    if let crate::ir::reexports::Operand::Value(v) = val {
                        if !self.reg_assignments.contains_key(&v.0) {
                            if let Some(slot) = self.state.get_slot(v.0) {
                                float_alloca_slots.insert(v.0, slot.0);
                            }
                        }
                    }
                }
                _ => {}
            }
            let lowered = super::isel::lower_instruction_typed_ss(
                inst,
                &self.reg_assignments,
                &float_alloca_slots,
                Some(&self.value_types),
                Some(&self.state.small_slot_values),
                &mut self.machinst_buf,
            );
            if lowered {
                self.machinst_buf_ir.push(inst.clone());
            }
            return lowered;
        }

        // Typed i128 route (the Store(other) / Load(other) census classes).
        // Mirrors the float route: an early, exact admission gate plus a
        // slot map that includes the VALUE home — the 16-byte i128 slot,
        // which the generic scan below would reject outright (i128 values
        // are never register-assigned, so a Load dest / Store val with no
        // RA entry trips the no-register rule before the isel could speak).
        if self.i128_typed_candidate(inst, folded_global_addrs) {
            let mut i128_slots = crate::common::fx_hash::FxHashMap::default();
            match inst {
                crate::ir::reexports::Instruction::Load { dest, ptr, .. } => {
                    if self.state.is_alloca(ptr.0) {
                        if let Some(slot) = self.state.get_slot(ptr.0) {
                            i128_slots.insert(ptr.0, slot.0);
                        }
                    }
                    if let Some(slot) = self.state.get_slot(dest.0) {
                        i128_slots.insert(dest.0, slot.0);
                    }
                }
                crate::ir::reexports::Instruction::Store { val, ptr, .. } => {
                    if self.state.is_alloca(ptr.0) {
                        if let Some(slot) = self.state.get_slot(ptr.0) {
                            i128_slots.insert(ptr.0, slot.0);
                        }
                    }
                    if let crate::ir::reexports::Operand::Value(v) = val {
                        if let Some(slot) = self.state.get_slot(v.0) {
                            i128_slots.insert(v.0, slot.0);
                        }
                    }
                }
                _ => {}
            }
            let lowered = super::isel::lower_instruction_typed_ss(
                inst,
                &self.reg_assignments,
                &i128_slots,
                Some(&self.value_types),
                Some(&self.state.small_slot_values),
                &mut self.machinst_buf,
            );
            if lowered {
                self.machinst_buf_ir.push(inst.clone());
            }
            return lowered;
        }

        // Reject instructions involving values we can't handle.
        // Allocas are allowed for Load/Store ptr (ISel handles via StackSlot).
        use crate::backend::liveness::for_each_operand_in_instruction;
        let ra = &self.reg_assignments;
        let state = &self.state;
        let mut reject = false;
        let is_load_store = matches!(
            inst,
            crate::ir::reexports::Instruction::Load { .. }
                | crate::ir::reexports::Instruction::Store { .. }
        );
        for_each_operand_in_instruction(inst, |op| {
            if let crate::ir::reexports::Operand::Value(v) = op {
                // Alloca operand VALUES (not ptrs) are still rejected
                if state.is_alloca(v.0) {
                    reject = true;
                    return;
                }
                if is_mi_unsafe_value(v.0, ra, state, &self.value_types) {
                    reject = true;
                    return;
                }
                if let Some(r) = ra.get(&v.0) {
                    if r.0 >= 20 {
                        reject = true;
                        return;
                    }
                }
                if !ra.contains_key(&v.0) && state.get_slot(v.0).is_none() {
                    reject = true;
                }
            }
        });
        // Load/Store ptr: allow allocas (ISel handles via StackSlot) and register ptrs.
        // Reject folded GEP ptrs, XMM ptrs, and ptrs with no register and no alloca.
        match inst {
            crate::ir::reexports::Instruction::Load { ptr, .. }
            | crate::ir::reexports::Instruction::Store { ptr, .. } => {
                // A pointer whose address is folded into a direct symbol(%rip)
                // operand by the default path has NO materialized address: the
                // GlobalAddr instruction was skipped. MachInst would emit a
                // memory operand through a register that is never loaded
                // (gzip: `movq %r15, (%r13)` with r13 never set -> SEGV).
                if folded_global_addrs.contains(&ptr.0) {
                    reject = true;
                }
                if state.folded_gep_values.contains(&ptr.0) {
                    reject = true;
                }
                // Over-aligned allocas (e.g. _Alignas(32) SIMD buffers): the
                // runtime address is (slot+align-1)&~(align-1), not the raw
                // slot. MachInst's lower_load/lower_store use the raw slot
                // address and would write to the WRONG location. The default
                // path handles OverAligned via emit_alloca_aligned_addr.
                if let Some(crate::backend::state::SlotAddr::OverAligned(_, _)) =
                    state.resolve_slot_addr(ptr.0)
                {
                    reject = true;
                }
                if let Some(r) = ra.get(&ptr.0) {
                    if r.0 >= 20 {
                        reject = true;
                    }
                }
                // Ptr must be: alloca, register, or stack slot (ISel loads to rcx)
                if !state.is_alloca(ptr.0)
                    && !ra.contains_key(&ptr.0)
                    && state.get_slot(ptr.0).is_none()
                {
                    reject = true;
                }
            }
            crate::ir::reexports::Instruction::GetElementPtr { base, offset, .. } => {
                // GetElementPtr does not encode the variable index's effective
                // signed width. Feeding an I32 -1 directly to 64-bit LEA turns
                // it into +0xffffffff (SQLite FTS5 porter Step1B2 crash).
                // Constant offsets remain MachInst-safe; variable offsets stay
                // on mature codegen until machine IR carries a typed index.
                if matches!(offset, crate::ir::reexports::Operand::Value(_)) {
                    reject = true;
                }
                // Over-aligned alloca bases need the aligned address; the
                // default path computes it, MachInst would use the raw slot.
                if state.is_alloca(base.0) {
                    if let Some(crate::backend::state::SlotAddr::OverAligned(_, _)) =
                        state.resolve_slot_addr(base.0)
                    {
                        reject = true;
                    }
                }
                // Allow alloca bases (ISel handles via AllocaAddr + leaq).
                // Non-alloca bases must be register-homed: ISel emits
                // `Lea { base: Vreg }` for a stack-homed pointer, and
                // resolve_stack_vregs cannot rewrite that to a SIB off the
                // slot — the slot holds the POINTER VALUE, not an address
                // to index. Fallback then re-emits with empty fold maps.
                if !state.is_alloca(base.0) && !ra.contains_key(&base.0) {
                    reject = true;
                }
                if let Some(r) = ra.get(&base.0) {
                    if r.0 >= 20 {
                        reject = true;
                    }
                }
            }
            // 64-bit Add is selected as Lea (base + index). A stack-homed
            // operand becomes MachReg::Vreg; Lea cannot encode it, and
            // substituting the slot as the Lea base would compute
            // `&slot + index` instead of `*slot + index`. The unresolvable
            // Vreg forces a default-path replay with empty GEP/global-addr
            // fold maps, which ICEs on values the skipped producers never
            // homed (kernel tracing_iter_reset: RELOC_HIDE ptr +
            // `__per_cpu_offset[cpu]` after an indexed-sym load).
            crate::ir::reexports::Instruction::BinOp {
                op: crate::ir::reexports::IrBinOp::Add,
                lhs,
                rhs,
                ty,
                ..
            } if ty.size() == 8 && !ty.is_float() => {
                let add_op_is_phys = |op: &crate::ir::reexports::Operand| match op {
                    crate::ir::reexports::Operand::Const(_) => true,
                    crate::ir::reexports::Operand::Value(v) => {
                        ra.get(&v.0).is_some_and(|r| r.0 < 20)
                    }
                };
                if !add_op_is_phys(lhs) || !add_op_is_phys(rhs) {
                    reject = true;
                }
            }
            _ => {}
        }
        // Dest must have a register for ALU/Shift/Neg/Not (two-address instructions).
        // Cmp/SetCC don't write to a GPR dest in a useful way (flags only).
        // Copy dest can be a stack slot (resolved to Mov with StackSlot).
        if let Some(dest) = inst.dest() {
            if state.is_alloca(dest.0) {
                reject = true;
            }
            if is_mi_unsafe_value(dest.0, ra, state, &self.value_types) {
                reject = true;
            }
            if let Some(r) = ra.get(&dest.0) {
                if r.0 >= 20 {
                    reject = true;
                }
                // Workaround: skip rbp-dest BinOps to avoid triggering a register
                // name bug in the existing codegen after MachInst cache invalidation.
                if r.0 == 6 {
                    reject = true;
                }
            }
            if !ra.contains_key(&dest.0) {
                match inst {
                    crate::ir::reexports::Instruction::Copy { .. } => {
                        if state.get_slot(dest.0).is_none() {
                            reject = true;
                        }
                    }
                    _ => {
                        reject = true;
                    }
                }
            }
        }
        if reject {
            return false;
        }

        // Build alloca slot map for Load/Store/GEP ISel
        let mut alloca_slots = crate::common::fx_hash::FxHashMap::default();
        match inst {
            crate::ir::reexports::Instruction::Load { ptr, .. }
            | crate::ir::reexports::Instruction::Store { ptr, .. } => {
                if self.state.is_alloca(ptr.0) {
                    if let Some(slot) = self.state.get_slot(ptr.0) {
                        alloca_slots.insert(ptr.0, slot.0);
                    }
                }
            }
            crate::ir::reexports::Instruction::GetElementPtr { base, .. } => {
                if self.state.is_alloca(base.0) {
                    if let Some(slot) = self.state.get_slot(base.0) {
                        alloca_slots.insert(base.0, slot.0);
                    }
                }
            }
            _ => {}
        }

        let lowered = super::isel::lower_instruction_typed_ss(
            inst,
            &self.reg_assignments,
            &alloca_slots,
            Some(&self.value_types),
            Some(&self.state.small_slot_values),
            &mut self.machinst_buf,
        );
        if lowered {
            self.machinst_buf_ir.push(inst.clone());
        }
        lowered
    }

    fn flush_machinst(&mut self) {
        if self.machinst_buf.is_empty() {
            // No machine instructions pending: nothing to emit. The IR shadow
            // (`machinst_buf_ir`) may still hold entries from no-op lowerings
            // — a coalesced Copy whose src and dest resolve to the same home
            // (value_to_reg(src) == value_to_reg(dest)) emits zero MachInsts
            // but is still recorded in the shadow. Drop the shadow too:
            // leaving it behind lets a later flush — possibly in a *different
            // function* after the boundary — replay stale IR whose values
            // have no homes there, tripping the operand_to_rax hard gate
            // (kernel drivers/tty/tty_io.c: tty_tiocmget replayed
            // tty_get_tiocm's tail `Copy v19 = v12`). A no-op window needs no
            // replay: the values are already at their destinations.
            self.machinst_buf_ir.clear();
            return;
        }

        use super::machinst::{MachInst, MachOperand, MachReg, OpSize};

        if std::env::var("CCC_MI_STREAM").is_ok() {
            eprintln!(
                "### MI-STREAM fn={} n={}",
                self.state.current_func_name,
                self.machinst_buf.len()
            );
            for mi in &self.machinst_buf {
                eprintln!("  {mi:?}");
            }
        }
        let resolved: Vec<MachInst> = self
            .machinst_buf
            .iter()
            .map(|inst| resolve_stack_vregs(inst, &self.reg_assignments, &self.state))
            .collect();

        let has_bad = resolved
            .iter()
            .any(|mi| has_unresolvable_vreg(mi, &self.reg_assignments));
        if has_bad {
            if std::env::var("CCC_MI_DEBUG").is_ok() {
                eprintln!(
                    "[MI-FALLBACK] {} instructions -> default path",
                    self.machinst_buf_ir.len()
                );
                for mi in &resolved {
                    if has_unresolvable_vreg(mi, &self.reg_assignments) {
                        eprintln!("[MI-FALLBACK]   unresolvable: {mi:?}");
                    }
                }
                for inst in &self.machinst_buf_ir {
                    eprintln!("[MI-FALLBACK]   ir: {inst:?}");
                }
            }
            // CORRECTNESS: never delete instructions. Re-emit the buffered IR
            // through the default (accumulator) codegen path, which handles
            // stack-backed values in every position. The buffer is a
            // straight-line sequence (flushed before calls and at block ends),
            // the register cache is still valid for the pre-buffer state, and
            // GEP folding is a pure optimization — an empty fold map keeps the
            // default path semantics-identical.
            let ir = std::mem::take(&mut self.machinst_buf_ir);
            // The main dispatch loop already advanced once for each buffered
            // instruction. Replay at those original points, then finish at
            // exactly the same point; advancing from the current endpoint
            // would double-count the whole fallback window and misalign later
            // call-spanning save/restore intervals.
            let replay_endpoint = self.state.current_program_point;
            let replay_len = u32::try_from(ir.len())
                .expect("MachInst replay window exceeds u32 program-point space");
            self.state.current_program_point = replay_endpoint
                .checked_sub(replay_len)
                .expect("MachInst replay exceeds emitted IR program points");
            for inst in ir {
                crate::backend::generation::generate_instruction(
                    self,
                    &inst,
                    &crate::common::fx_hash::FxHashMap::default(),
                    &crate::common::fx_hash::FxHashMap::default(),
                    &crate::common::fx_hash::FxHashMap::default(),
                    &crate::common::fx_hash::FxHashSet::default(),
                    &crate::common::fx_hash::FxHashSet::default(),
                    &crate::common::fx_hash::FxHashSet::default(),
                    &crate::common::fx_hash::FxHashMap::default(),
                );
                self.state.current_program_point += 1;
            }
            assert_eq!(
                self.state.current_program_point, replay_endpoint,
                "MachInst replay changed the IR program-point endpoint"
            );
            self.machinst_buf.clear();
            return;
        }

        super::machinst_emit::emit_machinsts(&resolved, &mut self.state.out);

        self.machinst_buf.clear();
        self.machinst_buf_ir.clear();
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    fn emit_reg_to_reg_move(&mut self, src: PhysReg, dest: PhysReg) {
        let s_name = phys_reg_name(src);
        let d_name = phys_reg_name(dest);
        self.state
            .out
            .emit_instr_reg_reg("    movq", s_name, d_name);
    }

    fn emit_acc_to_phys_reg(&mut self, dest: PhysReg) {
        let d_name = phys_reg_name(dest);
        self.state.out.emit_instr_reg_reg("    movq", "rax", d_name);
    }

    fn emit_pre_call_save_caller_regs(&mut self) {
        let point = self.state.current_program_point;
        for (&reg_id, &slot) in &self.caller_save_spill_slots {
            if let Some(intervals) = self.caller_save_intervals.get(&reg_id) {
                if intervals
                    .iter()
                    .any(|&(start, end)| start <= point && point <= end)
                {
                    let reg_name = phys_reg_name(PhysReg(reg_id));
                    self.state
                        .out
                        .emit_instr_reg_rbp("    movq", reg_name, slot.0);
                }
            }
        }
    }

    fn emit_post_call_restore_caller_regs(&mut self) {
        let point = self.state.current_program_point;
        for (&reg_id, &slot) in &self.caller_save_spill_slots {
            if let Some(intervals) = self.caller_save_intervals.get(&reg_id) {
                if intervals
                    .iter()
                    .any(|&(start, end)| start <= point && point <= end)
                {
                    let reg_name = phys_reg_name(PhysReg(reg_id));
                    self.state
                        .out
                        .emit_instr_rbp_reg("    movq", slot.0, reg_name);
                }
            }
        }
    }

    fn jump_mnemonic(&self) -> &'static str {
        "jmp"
    }
    fn trap_instruction(&self) -> &'static str {
        "ud2"
    }

    fn emit_branch_nonzero(&mut self, label: &str) {
        self.state.emit("    testq %rax, %rax");
        self.state.out.emit_jcc_label("    jne", label);
    }

    fn emit_jump_indirect(&mut self) {
        self.emit_retpoline_jump("rax");
    }

    fn emit_switch(
        &mut self,
        val: &Operand,
        cases: &[(i64, BlockId)],
        default: &BlockId,
        ty: IrType,
    ) {
        // 128-bit switches need an explicit high-half guard: the compare
        // chain and the jump table below operate on the low 64-bit half only
        // (%rax), so a case value representable in i64 matches only when the
        // high half is its sign extension — 0 for non-negative cases, -1 for
        // negative ones.  Without the guard, ((__int128)1 << 64) matched
        // `case 0` (gcc.c-torture execute/pr122943.c).
        if matches!(ty, IrType::I128 | IrType::U128) && !cases.is_empty() {
            let has_nonneg = cases.iter().any(|&(v, _)| v >= 0);
            let has_neg = cases.iter().any(|&(v, _)| v < 0);
            if has_neg && has_nonneg {
                // Mixed sign classes: two guarded chains.
                //   high == 0  -> non-negative chain
                //   high == -1 -> negative chain
                //   else       -> default
                self.emit_load_acc_pair(val); // %rax = low, %rdx = high
                let nonneg_chain = self.state.fresh_label("switch128_nonneg");
                let neg_chain = self.state.fresh_label("switch128_neg");
                self.state.out.emit_instr_imm_reg("    cmpq", 0, "rdx");
                self.state.out.emit_jcc_label("    je", &nonneg_chain);
                self.state.out.emit_instr_imm_reg("    cmpq", -1, "rdx");
                self.state.out.emit_jcc_label("    je", &neg_chain);
                self.emit_branch_to_block(*default);
                self.state.out.emit_named_label(&nonneg_chain);
                for &(case_val, target) in cases.iter().filter(|&&(v, _)| v >= 0) {
                    let label = target.as_label();
                    self.emit_switch_case_branch_128_low(case_val, &label);
                }
                self.emit_branch_to_block(*default);
                self.state.out.emit_named_label(&neg_chain);
                for &(case_val, target) in cases.iter().filter(|&&(v, _)| v < 0) {
                    let label = target.as_label();
                    self.emit_switch_case_branch_128_low(case_val, &label);
                }
                self.emit_branch_to_block(*default);
            } else {
                // Single sign class: one guard, then the low-half chain.
                let required_high: i64 = if has_neg { -1 } else { 0 };
                self.emit_load_acc_pair(val); // %rax = low, %rdx = high
                self.state
                    .out
                    .emit_instr_imm_reg("    cmpq", required_high, "rdx");
                self.state
                    .out
                    .emit_jcc_label("    jne", &default.as_label());
                for &(case_val, target) in cases {
                    let label = target.as_label();
                    self.emit_switch_case_branch_128_low(case_val, &label);
                }
                self.emit_branch_to_block(*default);
            }
            return;
        }
        self.emit_switch_native(val, cases, default, ty);
    }

    fn emit_switch_case_branch(&mut self, case_val: i64, label: &str, ty: IrType) {
        let use_32bit = matches!(
            ty,
            IrType::I32 | IrType::U32 | IrType::I16 | IrType::U16 | IrType::I8 | IrType::U8
        );
        if case_val == 0 {
            if use_32bit {
                self.state.emit("    testl %eax, %eax");
            } else {
                self.state.emit("    testq %rax, %rax");
            }
        } else if use_32bit {
            // Use 32-bit comparison to avoid sign-extension mismatch for int-sized values
            self.state
                .out
                .emit_instr_imm_reg("    cmpl", case_val as i32 as i64, "eax");
        } else if case_val >= i32::MIN as i64 && case_val <= i32::MAX as i64 {
            self.state
                .out
                .emit_instr_imm_reg("    cmpq", case_val, "rax");
        } else {
            self.state
                .out
                .emit_instr_imm_reg("    movabsq", case_val, "rcx");
            self.state.emit("    cmpq %rcx, %rax");
        }
        self.state.out.emit_jcc_label("    je", label);
    }

    fn emit_switch_jump_table(
        &mut self,
        val: &Operand,
        cases: &[(i64, BlockId)],
        default: &BlockId,
        ty: IrType,
    ) {
        use crate::backend::traits::build_jump_table;
        let (table, min_val, range) = build_jump_table(cases, default);
        let table_label = self.state.fresh_label("jt");
        let default_label = default.as_label();

        self.operand_to_rax(val);

        // For 32-bit switch types, sign-extend to 64-bit for the jump table indexing
        let use_32bit = matches!(
            ty,
            IrType::I32 | IrType::U32 | IrType::I16 | IrType::U16 | IrType::I8 | IrType::U8
        );
        if use_32bit {
            if ty.is_unsigned() {
                // Zero-extend: mov %eax, %eax clears upper 32 bits
                self.state.emit("    movl %eax, %eax");
            } else {
                // Sign-extend 32-bit to 64-bit
                self.state.emit("    cltq");
            }
        }

        if min_val != 0 {
            if min_val >= i32::MIN as i64 && min_val <= i32::MAX as i64 {
                self.state
                    .out
                    .emit_instr_imm_reg("    subq", min_val, "rax");
            } else {
                self.state
                    .out
                    .emit_instr_imm_reg("    movabsq", min_val, "rcx");
                self.state.emit("    subq %rcx, %rax");
            }
        }
        if (range as i64) >= i32::MIN as i64 && (range as i64) <= i32::MAX as i64 {
            self.state
                .out
                .emit_instr_imm_reg("    cmpq", range as i64, "rax");
        } else {
            self.state
                .out
                .emit_instr_imm_reg("    movabsq", range as i64, "rcx");
            self.state.emit("    cmpq %rcx, %rax");
        }
        self.state.out.emit_jcc_label("    jae", &default_label);

        // Kernel / non-PIC code model: emit the GCC-canonical ABSOLUTE jump
        // table — `jmp *jt_table(,%rax,8)` (R_X86_64_32S against .rodata)
        // plus `.quad` entries (R_X86_64_64 with absolute target offsets).
        // This is the only shape the kernel's objtool can validate: its
        // add_jump_table() reads each entry's RELA addend as an absolute
        // instruction offset, but PIC-style `.long target - table` entries
        // carry position-baked addends (the GAS convention adds the entry's
        // own offset, so S + A - P resolves at link time), which objtool
        // then resolves mid-instruction and rejects the object with
        // "can't find switch jump table" (observed on
        // arch/x86/kernel/platform-quirks.o). GCC avoids this entirely by
        // using absolute tables under -mcmodel=kernel/-fno-pic, so mirror
        // that shape here.
        let absolute_table = self.state.code_model_kernel || !self.state.pic_mode;
        if absolute_table {
            self.state
                .emit_fmt(format_args!("    jmpq *{}(,%rax,8)", table_label));
        } else {
            // PIC: RIP-relative table base + PC32 differences. Correct for
            // user-space PIC (no objtool validation there).
            self.state
                .out
                .emit_instr_sym_base_reg("    leaq", &table_label, "rip", "rcx");
            self.state.emit("    movslq (%rcx,%rax,4), %rdx");
            self.state.emit("    addq %rcx, %rdx");
            // Switch-table dispatch is an indirect branch: it needs the same
            // retpoline protection as calls (Spectre v2 via the BTB applies to
            // any indirect JMP; kernel objtool flags naked ones in vDSO too).
            self.emit_retpoline_jump("rdx");
        }

        self.state.emit(".section .rodata");
        self.state.emit(if absolute_table {
            ".align 8"
        } else {
            ".align 4"
        });
        self.state.out.emit_named_label(&table_label);
        for target in &table {
            let target_label = target.as_label();
            if absolute_table {
                self.state
                    .emit_fmt(format_args!("    .quad {}", target_label));
            } else {
                self.state
                    .emit_fmt(format_args!("    .long {} - {}", target_label, table_label));
            }
        }
        let sect = self.state.current_text_section.clone();
        self.state
            .emit_fmt(format_args!(".section {},\"ax\",@progbits", sect));

        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    // ---- Standard trait methods (kept inline) ----
    fn emit_load_operand(&mut self, op: &Operand) {
        self.operand_to_rax(op)
    }
    fn emit_store_result(&mut self, dest: &Value) {
        self.store_rax_to(dest)
    }
    fn supports_global_addr_fold(&self) -> bool {
        true
    }

    fn supports_global_addr_remat(&self) -> bool {
        true
    }

    fn emit_rematerialized_global_addr(
        &mut self,
        dest: &Value,
        sym: &str,
        offset: &Operand,
        subtract: bool,
    ) -> bool {
        self.emit_rematerialized_global_addr_impl(dest, sym, offset, subtract)
    }

    /// Session 28: x86-64 SIB indexed addressing (mirrors session-27 i686):
    /// `off(%base,%idx,scale)` / `sym(,%idx,scale)` fold variable-offset GEPs
    /// into one memory operand.  The prologue wires base AND index liveness
    /// (collect_folded_gep_links_all) so both registers survive to the access.
    fn supports_indexed_addr(&self) -> bool {
        std::env::var_os("CCC_NO_X64_SIB").is_none()
    }

    fn supports_indexed_sym_base(&self) -> bool {
        // In PIE/PIC x86 cannot combine RIP-relative addressing and an index
        // in one memory operand. The symbol hook still supports every legal
        // direct symbol by rebuilding its base in the reserved address scratch
        // and retaining the scale in the consuming SIB operand.
        std::env::var_os("CCC_NO_X64_SIB").is_none()
    }

    fn const_offset_fold_reg_base_ok(&self, base: &Value) -> bool {
        // Register-base const-offset folds consume the base at the Load/Store
        // position (RA-invisible): sound only with the folded-base liveness
        // links wired into register allocation (prologue passes
        // collect_gep_fold_base_links).  When that extension is disabled the
        // folds must be refused too — the two are one contract.
        if std::env::var_os("CCC_NO_FOLDED_INDEX_LIVENESS").is_some() {
            return false;
        }
        match self.reg_assignments.get(&base.0) {
            Some(&phys) if !is_xmm_reg(phys) => {
                // Exclude the emitter's private scratch set for the fold
                // paths: %rdx (PhysReg 16, emit_save_acc staging) and %r11
                // (PhysReg 10, operand_to_rax staging / indirect-target
                // scratch).  %rax/%rcx are not allocatable homes to begin
                // with.  Every other GPR home is safe: the operand staging
                // (operand_to_rax, xmm staging) only touches that set.
                !matches!(phys.0, 10 | 16)
            }
            _ => false,
        }
    }

    fn emit_reg_to_acc(&mut self, reg: PhysReg) {
        // The trait DEFAULT for this hook is a NO-OP ("backends override").
        // Without the override, default emit_gep fallbacks that stage a
        // register-resident base through the accumulator silently emit
        // nothing and the address math reads a stale scratch (the exact i686
        // emit_leaq_base_index bug class, fixed there in session 20).
        let name = phys_reg_name(reg);
        self.state
            .emit_fmt(format_args!("    movq %{}, %rax", name));
        self.state.reg_cache.invalidate_acc();
    }

    fn emit_reg_to_secondary(&mut self, reg: PhysReg) {
        // Single home->secondary move for the default emit_gep's general
        // path: staging a register-resident base must not pass through the
        // accumulator (which may still hold the live GEP offset).  The
        // secondary cache cannot be updated (the hook carries no value id),
        // so invalidate it — matches the conservative emit_reg_to_addr
        // precedent.
        let name = phys_reg_name(reg);
        self.state
            .emit_fmt(format_args!("    movq %{}, %rcx", name));
        self.state.reg_cache.invalidate_sec();
    }

    fn emit_gep_reg_const(&mut self, dest: &Value, base: &Value, offset: i64) -> bool {
        // Register-resident base + constant offset → `leaq off(%base), %dest`.
        // Without this hook the default emit_gep silently drops the base for
        // slot-less bases: both its const and general branches key on
        // resolve_slot_addr(base), so a rematerialised folded GEP computed
        // `offset + stale scratch` (zlib-ng gen_bitlen corrupt-pointer crash:
        // tree/bl_count fields read and written through garbage addresses).
        let Some(&br) = self.reg_assignments.get(&base.0) else {
            return false;
        };
        if is_xmm_reg(br) || offset < i32::MIN as i64 || offset > i32::MAX as i64 {
            return false;
        }
        let b_name = phys_reg_name(br);
        if let Some(&dr) = self.reg_assignments.get(&dest.0) {
            if !is_xmm_reg(dr) {
                let d_name = phys_reg_name(dr);
                if offset == 0 {
                    if br != dr {
                        self.state
                            .emit_fmt(format_args!("    movq %{}, %{}", b_name, d_name));
                    }
                } else {
                    self.state.emit_fmt(format_args!(
                        "    leaq {}(%{}), %{}",
                        offset, b_name, d_name
                    ));
                }
                self.state.reg_cache.invalidate_acc();
                return true;
            }
        }
        // Slot-only dest: stage through %rax, then store to the home.
        if offset == 0 {
            self.state
                .emit_fmt(format_args!("    movq %{}, %rax", b_name));
        } else {
            self.state
                .emit_fmt(format_args!("    leaq {}(%{}), %rax", offset, b_name));
        }
        self.state.reg_cache.set_acc(dest.0, false);
        self.emit_store_result(dest);
        true
    }

    fn emit_call_store_f128_result(&mut self, _dest: &Value) {
        unreachable!("x86 uses custom emit_call_store_result for F128")
    }

    fn emit_copy_value(&mut self, dest: &Value, src: &Operand) {
        // Flag-fusion forwarding: this copy's destination is never read (the
        // fused consumer uses the live Cmp flags), and its source is the
        // never-materialized boolean — emitting it would read a stale register.
        if self.fused_forward_dests.contains(&dest.0) {
            return;
        }
        // Handle vector Copy instructions: %dest_vec = copy %src_vec
        // Vectors are stored directly in stack slots (not as pointers), so we copy
        // with ymm/xmm registers instead of scalar movq.
        if let Operand::Value(v) = src {
            if self.state.vector_values.contains(&v.0) {
                let debug = std::env::var("LCCC_DEBUG_PROTECT").is_ok();
                let is_128 = self.state.vector128_values.contains(&v.0);
                if debug {
                    eprintln!(
                        "[COPY-VEC] Copying {}-bit vector SSA {} to SSA {}",
                        if is_128 { 128 } else { 256 },
                        v.0,
                        dest.0
                    );
                }

                // Coalesced loop-carried vectors already occupy the same
                // physical SIMD register; their entry/backedge copies vanish.
                let src_reg = self
                    .reg_assignments
                    .get(&v.0)
                    .copied()
                    .filter(|r| is_xmm_reg(*r));
                let dest_reg = self
                    .reg_assignments
                    .get(&dest.0)
                    .copied()
                    .filter(|r| is_xmm_reg(*r));
                if let (Some(s), Some(d)) = (src_reg, dest_reg) {
                    let held = if is_128 {
                        phys_reg_name(s)
                    } else {
                        phys_reg_name_256(s)
                    };
                    let target = if is_128 {
                        phys_reg_name(d)
                    } else {
                        phys_reg_name_256(d)
                    };
                    if s != d {
                        let mov = if is_128 { "movdqa" } else { "vmovdqa" };
                        self.state
                            .emit_fmt(format_args!("    {} %{}, %{}", mov, held, target));
                    }
                    self.state.vec_live_regs.insert(dest.0, target);
                    self.state.vector_values.insert(dest.0);
                    if is_128 {
                        self.state.vector128_values.insert(dest.0);
                    }
                    return;
                }

                // Mixed assigned/stack and pure stack copies use the same
                // width-aware load/store helpers as intrinsic operands. This
                // avoids interpreting register-held vector bits as an address.
                if is_128 {
                    self.sse_load_arg(src, "xmm0");
                    self.sse_store_dest(dest, "xmm0");
                } else {
                    self.avx_load_arg(src);
                    self.avx_store_dest(dest);
                }
                self.state.vector_values.insert(dest.0);
                if is_128 {
                    self.state.vector128_values.insert(dest.0);
                }
                return;
            }

            if self.state.f128_direct_slots.contains(&v.0) {
                if let (Some(src_slot), Some(dest_slot)) =
                    (self.state.get_slot(v.0), self.state.get_slot(dest.0))
                {
                    self.state.out.emit_instr_rbp("    fldt", src_slot.0);
                    self.state.out.emit_instr_rbp("    fstpt", dest_slot.0);
                    self.state.out.emit_instr_rbp("    fldt", dest_slot.0);
                    self.state.emit("    subq $8, %rsp");
                    self.state.emit("    fstpl (%rsp)");
                    self.state.emit("    popq %rax");
                    self.state.reg_cache.set_acc(dest.0, false);
                    self.state.f128_direct_slots.insert(dest.0);
                    return;
                }
            }
        }

        let dest_phys = self.dest_reg(dest);
        let src_phys = self.operand_reg(src);

        match (dest_phys, src_phys) {
            (Some(d), Some(s)) => {
                if d.0 != s.0 {
                    let d_name = phys_reg_name(d);
                    let s_name = phys_reg_name(s);
                    if is_xmm_reg(d) && is_xmm_reg(s) {
                        // XMM → XMM copy: movapd moves all 128 bits (upper 64
                        // are dead for scalar F64/F32) without the reg-reg
                        // movsd merge form's false dependency on the old
                        // destination value.
                        self.state
                            .emit_fmt(format_args!("    movapd %{}, %{}", s_name, d_name));
                    } else {
                        self.state
                            .out
                            .emit_instr_reg_reg("    movq", s_name, d_name);
                    }
                }
                self.state.reg_cache.invalidate_acc();
            }
            (Some(d), None) => {
                // FP destination: load the operand straight into the XMM
                // register (rip-relative constant load / movsd from slot)
                // instead of bouncing the bit pattern through %rax.
                if is_xmm_reg(d) {
                    let d_name = phys_reg_name(d);
                    let ty = match src {
                        Operand::Const(IrConst::F32(_)) => IrType::F32,
                        _ => IrType::F64,
                    };
                    self.emit_fp_operand_to_xmm(src, ty, d_name);
                    self.state.reg_cache.invalidate_acc();
                    return;
                }
                // A constant copied into a register-allocated dest is emitted
                // directly (movq $imm, %reg / xorl for zero) instead of the
                // relay `movq $imm, %rax; movq %rax, %reg` — the short-circuit
                // merge value (`x = 1`) and loop preambles pay this copy per
                // execution. Values (slots / accumulator-cached) keep the rax
                // relay (their width-aware load semantics must be preserved).
                if matches!(src, Operand::Const(_)) {
                    self.operand_to_callee_reg(src, d);
                } else {
                    self.operand_to_rax(src);
                    let d_name = phys_reg_name(d);
                    self.state.out.emit_instr_reg_reg("    movq", "rax", d_name);
                }
                self.state.reg_cache.invalidate_acc();
            }
            _ => {
                // XMM source → stack-slot dest: movss/movsd directly, no GPR
                // relay. Both F32 and F64 values can be XMM-allocated
                // (is_scalar_fp covers both), so the store width follows the
                // value type. %rax is untouched, so the accumulator cache
                // stays valid.
                if let Operand::Value(v) = src {
                    if let Some(s) = src_phys {
                        if is_xmm_reg(s)
                            && !self.state.is_alloca(dest.0)
                            && !self.state.vector_values.contains(&dest.0)
                        {
                            if let Some(slot) = self.state.get_slot(dest.0) {
                                // Width-consistent store: an XMM-allocated F32
                                // value must spill with movss (4 bytes) — movsd
                                // would write 8 bytes and, into a 4-byte small
                                // slot, clobber the neighbour. F64 keeps movsd.
                                let ty = self.value_types.get(&v.0).copied().unwrap_or(IrType::F64);
                                let mov = if ty == IrType::F32 {
                                    "    movss"
                                } else {
                                    "    movsd"
                                };
                                self.state
                                    .out
                                    .emit_instr_reg_rbp(mov, phys_reg_name(s), slot.0);
                                return;
                            }
                        }
                    }
                }
                self.operand_to_rax(src);
                self.store_rax_to(dest);
            }
        }
    }

    // ---- Inline asm (kept inline - has extra logic) ----
    fn emit_inline_asm(
        &mut self,
        template: &str,
        outputs: &[(String, Value, Option<String>)],
        inputs: &[(String, Operand, Option<String>)],
        clobbers: &[String],
        operand_types: &[IrType],
        goto_labels: &[(String, BlockId)],
        input_symbols: &[Option<String>],
    ) {
        // Lazy flush: inline asm may clobber XMM registers or read slots.
        self.flush_pending_vec_store_impl();
        emit_inline_asm_common(
            self,
            template,
            outputs,
            inputs,
            clobbers,
            operand_types,
            goto_labels,
            input_symbols,
        );
        self.emit_callee_saved_clobber_annotations(clobbers);
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    fn emit_inline_asm_with_segs(
        &mut self,
        template: &str,
        outputs: &[(String, Value, Option<String>)],
        inputs: &[(String, Operand, Option<String>)],
        clobbers: &[String],
        operand_types: &[IrType],
        goto_labels: &[(String, BlockId)],
        input_symbols: &[Option<String>],
        seg_overrides: &[AddressSpace],
    ) {
        // Lazy flush: inline asm may clobber XMM registers or read slots.
        self.flush_pending_vec_store_impl();
        crate::backend::inline_asm::emit_inline_asm_common_impl(
            self,
            template,
            outputs,
            inputs,
            clobbers,
            operand_types,
            goto_labels,
            input_symbols,
            seg_overrides,
        );
        self.emit_callee_saved_clobber_annotations(clobbers);
        self.state.reg_cache.invalidate_all();
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
    }

    fn flush_pending_vec_store(&mut self) {
        // Boundary hook (block ends, calls, inline asm, non-intrinsic
        // instructions): a VLFOLD-elided load that has not met its consumer
        // is materialised first so the mechanism is sound by construction.
        self.materialize_pending_memfold();
        self.flush_pending_vec_store_impl();
    }

    // ---- Intrinsics (kept inline - has extra logic) ----
    fn emit_intrinsic(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        self.emit_intrinsic_impl(dest, op, dest_ptr, args);
        // NOTE: do NOT invalidate the vector peephole here. The flag set by the
        // final avx_store_dest must survive into the NEXT intrinsic's operand
        // loads (e.g. movemask right after a compare). It is cleared by every
        // actual load, call, inline asm, atomic, memcpy, and block boundary.
        self.state.reg_cache.invalidate_all();
    }

    fn emit_reg_to_addr(&mut self, reg: PhysReg) {
        self.state.reg_cache.invalidate_sec();
        self.state
            .out
            .emit_instr_reg_reg("    movq", phys_reg_name(reg), "rcx");
    }

    fn function_alignment_log2(&self) -> Option<u32> {
        match self.function_alignment {
            0 | 1 => None,
            n => Some(n.trailing_zeros()),
        }
    }

    // All remaining methods delegate to self.method_name_impl(args...)
    delegate_to_impl! {
        // CCC_ENABLE_VECREG live-out flush (default no-op on other backends)
        fn flush_vecreg_liveout(&mut self) => flush_vecreg_liveout_impl;
        // prologue
        fn calculate_stack_space(&mut self, func: &IrFunction) -> i64 => calculate_stack_space_impl;
        fn aligned_frame_size(&self, raw_space: i64) -> i64 => aligned_frame_size_impl;
        fn emit_prologue(&mut self, func: &IrFunction, frame_size: i64) => emit_prologue_impl;
        fn emit_epilogue(&mut self, frame_size: i64) => emit_epilogue_impl;
        fn emit_store_params(&mut self, func: &IrFunction) => emit_store_params_impl;
        fn emit_param_ref(&mut self, dest: &Value, param_idx: usize, ty: IrType) => emit_param_ref_impl;
        fn emit_epilogue_and_ret(&mut self, frame_size: i64) => emit_epilogue_and_ret_impl;
        fn store_instr_for_type(&self, ty: IrType) -> &'static str => store_instr_for_type_impl;
        fn load_instr_for_type(&self, ty: IrType) -> &'static str => load_instr_for_type_impl;
        // memory
        fn emit_store(&mut self, val: &Operand, ptr: &Value, ty: IrType) => emit_store_impl;
        fn emit_load(&mut self, dest: &Value, ptr: &Value, ty: IrType) => emit_load_impl;
        fn emit_store_with_const_offset(&mut self, val: &Operand, base: &Value, offset: i64, ty: IrType) => emit_store_with_const_offset_impl;
        fn emit_load_with_const_offset(&mut self, dest: &Value, base: &Value, offset: i64, ty: IrType) => emit_load_with_const_offset_impl;
        fn emit_load_indexed(&mut self, dest: &Value, base: &Value, index: &Value, shift: u8, disp: i64, ty: IrType) -> bool => emit_load_indexed_impl;
        fn emit_store_indexed(&mut self, val: &Operand, base: &Value, index: &Value, shift: u8, disp: i64, ty: IrType) -> bool => emit_store_indexed_impl;
        fn emit_load_indexed_sym(&mut self, dest: &Value, sym: &str, index: &Value, shift: u8, disp: i64, ty: IrType) -> bool => emit_load_indexed_sym_impl;
        fn emit_store_indexed_sym(&mut self, val: &Operand, sym: &str, index: &Value, shift: u8, disp: i64, ty: IrType) -> bool => emit_store_indexed_sym_impl;
        fn emit_typed_store_to_slot(&mut self, instr: &'static str, ty: IrType, slot: StackSlot) => emit_typed_store_to_slot_impl;
        fn emit_typed_load_from_slot(&mut self, instr: &'static str, slot: StackSlot) => emit_typed_load_from_slot_impl;
        fn emit_save_acc(&mut self) => emit_save_acc_impl;
        fn emit_load_ptr_from_slot(&mut self, slot: StackSlot, val_id: u32) => emit_load_ptr_from_slot_impl;
        fn emit_typed_store_indirect(&mut self, instr: &'static str, ty: IrType) => emit_typed_store_indirect_impl;
        fn emit_typed_load_indirect(&mut self, instr: &'static str) => emit_typed_load_indirect_impl;
        fn emit_add_offset_to_addr_reg(&mut self, offset: i64) => emit_add_offset_to_addr_reg_impl;
        fn emit_slot_addr_to_secondary(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_slot_addr_to_secondary_impl;
        fn emit_add_secondary_to_acc(&mut self) => emit_add_secondary_to_acc_impl;
        fn emit_gep_direct_const(&mut self, slot: StackSlot, offset: i64) => emit_gep_direct_const_impl;
        fn emit_gep_indirect_const(&mut self, slot: StackSlot, offset: i64, val_id: u32) => emit_gep_indirect_const_impl;
        fn emit_gep_add_const_to_acc(&mut self, offset: i64) => emit_gep_add_const_to_acc_impl;
        fn emit_leaq_base_index(&mut self, base_reg: PhysReg, index_reg: PhysReg, dest: &Value, dest_phys: Option<PhysReg>) => emit_leaq_base_index_impl;
        fn emit_leaq_sym_index(&mut self, dest: &Value, sym: &str, index: &Value, shift: u8, disp: i64) -> bool => emit_leaq_sym_index_impl;
        fn emit_add_imm_to_acc(&mut self, imm: i64) => emit_add_imm_to_acc_impl;
        fn emit_round_up_acc_to_16(&mut self) => emit_round_up_acc_to_16_impl;
        fn emit_sub_sp_by_acc(&mut self) => emit_sub_sp_by_acc_impl;
        fn emit_mov_sp_to_acc(&mut self) => emit_mov_sp_to_acc_impl;
        fn emit_mov_acc_to_sp(&mut self) => emit_mov_acc_to_sp_impl;
        fn emit_align_acc(&mut self, align: usize) => emit_align_acc_impl;
        fn emit_memcpy_load_dest_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_memcpy_load_dest_addr_impl;
        fn emit_memcpy_load_src_addr(&mut self, slot: StackSlot, is_alloca: bool, val_id: u32) => emit_memcpy_load_src_addr_impl;
        fn emit_alloca_aligned_addr(&mut self, slot: StackSlot, val_id: u32) => emit_alloca_aligned_addr_impl;
        fn emit_alloca_aligned_addr_to_acc(&mut self, slot: StackSlot, val_id: u32) => emit_alloca_aligned_addr_to_acc_impl;
        fn emit_acc_to_secondary(&mut self) => emit_acc_to_secondary_impl;
        fn emit_memcpy_store_dest_from_acc(&mut self) => emit_memcpy_store_dest_from_acc_impl;
        fn emit_memcpy_store_src_from_acc(&mut self) => emit_memcpy_store_src_from_acc_impl;
        fn emit_memcpy_impl(&mut self, size: usize) => emit_memcpy_impl_impl;
        fn emit_inline_memcpy_call(&mut self, dest: &Operand, src: &Operand, size: usize) => emit_inline_memcpy_call_impl;
        fn emit_inline_memmove_call(&mut self, dest: &Operand, src: &Operand, size: usize) => emit_inline_memmove_call_impl;
        fn emit_seg_load(&mut self, dest: &Value, ptr: &Value, ty: IrType, seg: AddressSpace) => emit_seg_load_impl;
        fn emit_seg_load_const_addr(&mut self, dest: &Value, addr: i64, ty: IrType, seg: AddressSpace) -> bool => emit_seg_load_const_addr_impl;
        fn emit_seg_store_const_addr(&mut self, val: &Operand, addr: i64, ty: IrType, seg: AddressSpace) -> bool => emit_seg_store_const_addr_impl;
        fn emit_seg_load_symbol(&mut self, dest: &Value, sym: &str, ty: IrType, seg: AddressSpace) => emit_seg_load_symbol_impl;
        fn emit_seg_store(&mut self, val: &Operand, ptr: &Value, ty: IrType, seg: AddressSpace) => emit_seg_store_impl;
        fn emit_seg_store_symbol(&mut self, val: &Operand, sym: &str, ty: IrType, seg: AddressSpace) => emit_seg_store_symbol_impl;
        // alu
        fn emit_float_neg(&mut self, ty: IrType) => emit_float_neg_impl;
        fn emit_int_neg(&mut self, ty: IrType) => emit_int_neg_impl;
        fn emit_int_not(&mut self, ty: IrType) => emit_int_not_impl;
        fn emit_int_clz(&mut self, ty: IrType) => emit_int_clz_impl;
        fn emit_int_ctz(&mut self, ty: IrType) => emit_int_ctz_impl;
        fn emit_int_bswap(&mut self, ty: IrType) => emit_int_bswap_impl;
        fn emit_int_popcount(&mut self, ty: IrType) => emit_int_popcount_impl;
        fn emit_int_binop(&mut self, dest: &Value, op: IrBinOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_int_binop_impl;
        fn emit_fused_mul_add(&mut self, mul_dest: &Value, mul_lhs: &Operand, mul_rhs: &Operand, acc: &Operand, add_dest: &Value, ty: IrType) => emit_fused_mul_add_impl;
        fn emit_copy_i128(&mut self, dest: &Value, src: &Operand) => emit_copy_i128_impl;
        // comparison
        fn emit_f128_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand) => emit_f128_cmp_impl;
        fn emit_float_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_float_cmp_impl;
        fn emit_int_cmp(&mut self, dest: &Value, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_int_cmp_impl;
        fn emit_fused_cmp_branch(&mut self, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType, true_label: &str, false_label: &str) => emit_fused_cmp_branch_impl;
        fn emit_fused_cmp_branch_blocks(&mut self, op: IrCmpOp, lhs: &Operand, rhs: &Operand, ty: IrType, true_block: BlockId, false_block: BlockId) => emit_fused_cmp_branch_blocks_impl;
        fn emit_fused_bit_test_branch_blocks(&mut self, base: &Operand, index: &Operand, ty: IrType, true_block: BlockId, false_block: BlockId) => emit_fused_bit_test_branch_blocks_impl;
        fn emit_cond_branch_blocks(&mut self, cond: &Operand, true_block: BlockId, false_block: BlockId) => emit_cond_branch_blocks_impl;
        fn emit_select(&mut self, dest: &Value, cond: &Operand, true_val: &Operand, false_val: &Operand, ty: IrType) => emit_select_impl;
        // calls
        fn call_abi_config(&self) -> CallAbiConfig => call_abi_config_impl;
        fn emit_call_compute_stack_space(&self, arg_classes: &[CallArgClass], arg_types: &[IrType], struct_arg_aligns: &[Option<usize>], struct_arg_is_f128_sse: &[bool]) -> usize => emit_call_compute_stack_space_impl;
        fn emit_call_spill_fptr(&mut self, func_ptr: &Operand) => emit_call_spill_fptr_impl;
        fn emit_call_stack_args(&mut self, args: &[Operand], arg_classes: &[CallArgClass], arg_types: &[IrType], stack_arg_space: usize, fptr_spill: usize, f128_temp_space: usize, struct_arg_aligns: &[Option<usize>], struct_arg_is_f128_sse: &[bool]) -> i64 => emit_call_stack_args_impl;
        fn emit_call_reg_args(&mut self, args: &[Operand], arg_classes: &[CallArgClass], arg_types: &[IrType], total_sp_adjust: i64, f128_temp_space: usize, stack_arg_space: usize, struct_arg_riscv_float_classes: &[Option<crate::common::types::RiscvFloatClass>]) => emit_call_reg_args_impl;
        fn emit_call_instruction(&mut self, direct_name: Option<&str>, func_ptr: Option<&Operand>, indirect: bool, stack_arg_space: usize) => emit_call_instruction_impl;
        fn emit_call_cleanup(&mut self, stack_arg_space: usize, f128_temp_space: usize, indirect: bool) => emit_call_cleanup_impl;
        fn set_call_ret_eightbyte_classes(&mut self, classes: &[crate::common::types::EightbyteClass]) => set_call_ret_eightbyte_classes_impl;
        fn set_call_ret_is_f128_sse(&mut self, is_f128: bool) => set_call_ret_is_f128_sse_impl;
        fn emit_call_store_result(&mut self, dest: &Value, return_type: IrType) => emit_call_store_result_impl;
        fn emit_call_store_i128_result(&mut self, dest: &Value) => emit_call_store_i128_result_impl;
        fn emit_call_move_f32_to_acc(&mut self) => emit_call_move_f32_to_acc_impl;
        fn emit_call_move_f64_to_acc(&mut self) => emit_call_move_f64_to_acc_impl;
        // globals
        fn emit_global_addr(&mut self, dest: &Value, name: &str) => emit_global_addr_impl;
        fn emit_tls_global_addr(&mut self, dest: &Value, name: &str) => emit_tls_global_addr_impl;
        fn emit_global_addr_absolute(&mut self, dest: &Value, name: &str) => emit_global_addr_absolute_impl;
        fn emit_vector_const_rodata(&mut self) => emit_vector_const_rodata_impl;
        fn emit_global_load_rip_rel(&mut self, dest: &Value, sym: &str, ty: IrType) => emit_global_load_rip_rel_impl;
        fn emit_global_store_rip_rel(&mut self, val: &Operand, sym: &str, ty: IrType) => emit_global_store_rip_rel_impl;
        fn emit_label_addr(&mut self, dest: &Value, label: &str) => emit_label_addr_impl;
        // cast
        fn emit_cast_instrs(&mut self, from_ty: IrType, to_ty: IrType) => emit_cast_instrs_impl;
        fn emit_cast(&mut self, dest: &Value, src: &Operand, from_ty: IrType, to_ty: IrType) => emit_cast_impl;
        // variadic
        fn emit_va_arg(&mut self, dest: &Value, va_list_ptr: &Value, result_ty: IrType) => emit_va_arg_impl;
        fn emit_va_arg_struct(&mut self, dest_ptr: &Value, va_list_ptr: &Value, size: usize, align: usize) => emit_va_arg_struct_impl;
        fn emit_va_arg_struct_ex(&mut self, dest_ptr: &Value, va_list_ptr: &Value, size: usize, align: usize, eightbyte_classes: &[crate::common::types::EightbyteClass]) => emit_va_arg_struct_ex_impl;
        fn emit_va_start(&mut self, va_list_ptr: &Value) => emit_va_start_impl;
        fn emit_va_copy(&mut self, dest_ptr: &Value, src_ptr: &Value) => emit_va_copy_impl;
        // returns
        fn emit_return(&mut self, val: Option<&Operand>, frame_size: i64) => emit_return_impl;
        fn current_return_type(&self) -> IrType => current_return_type_impl;
        fn emit_return_f32_to_reg(&mut self) => emit_return_f32_to_reg_impl;
        fn emit_return_f64_to_reg(&mut self) => emit_return_f64_to_reg_impl;
        fn emit_return_i128_to_regs(&mut self) => emit_return_i128_to_regs_impl;
        fn emit_get_return_f64_second(&mut self, dest: &Value) => emit_get_return_f64_second_impl;
        fn emit_set_return_f64_second(&mut self, src: &Operand) => emit_set_return_f64_second_impl;
        fn emit_get_static_chain(&mut self, dest: &Value) => emit_get_static_chain_impl;
        fn emit_set_static_chain(&mut self, src: &Operand) => emit_set_static_chain_impl;
        fn emit_init_trampoline(&mut self, buffer: &Value, chain: &Operand, func: &str) => emit_init_trampoline_impl;
        fn emit_nonlocal_goto_save(&mut self, frame: &Value, rbp_off: i64, rsp_off: i64) => emit_nonlocal_goto_save_impl;
        fn emit_nonlocal_goto(&mut self, chain: &Operand, up: usize, rbp_off: i64, rsp_off: i64, label: &str) => emit_nonlocal_goto_impl;
        fn emit_get_return_f32_second(&mut self, dest: &Value) => emit_get_return_f32_second_impl;
        fn emit_set_return_f32_second(&mut self, src: &Operand) => emit_set_return_f32_second_impl;
        fn emit_return_f128_to_reg(&mut self) => emit_return_f128_to_reg_impl;
        fn emit_return_int_to_reg(&mut self) => emit_return_int_to_reg_impl;
        fn emit_get_return_f128_second(&mut self, dest: &Value) => emit_get_return_f128_second_impl;
        fn emit_set_return_f128_second(&mut self, src: &Operand) => emit_set_return_f128_second_impl;
        // atomics
        fn emit_pgo_counter_inc(&mut self, name: &str, offset: i64, atomic: bool) => emit_pgo_counter_inc_impl;
        fn emit_pgo_counter_nop(&mut self, name: &str, offset: i64, atomic: bool) => emit_pgo_counter_nop_impl;
        fn emit_atomic_inc(&mut self, ptr: &Operand, offset: i64, ty: IrType, ordering: AtomicOrdering) => emit_atomic_inc_impl;
        fn emit_atomic_rmw(&mut self, dest: &Value, op: AtomicRmwOp, ptr: &Operand, val: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_rmw_impl;
        fn emit_atomic_cmpxchg(&mut self, dest: &Value, ptr: &Operand, expected: &Operand, desired: &Operand, ty: IrType, success_ordering: AtomicOrdering, failure_ordering: AtomicOrdering, returns_bool: bool) => emit_atomic_cmpxchg_impl;
        fn emit_atomic_load(&mut self, dest: &Value, ptr: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_load_impl;
        fn emit_atomic_store(&mut self, ptr: &Operand, val: &Operand, ty: IrType, ordering: AtomicOrdering) => emit_atomic_store_impl;
        fn emit_fence(&mut self, ordering: AtomicOrdering) => emit_fence_impl;
        // float ops
        fn emit_float_binop_mnemonic(&self, op: FloatOp) -> &'static str => emit_float_binop_mnemonic_impl;
        fn emit_unaryop(&mut self, dest: &Value, op: IrUnaryOp, src: &Operand, ty: IrType) => emit_unaryop_impl;
        fn emit_float_binop(&mut self, dest: &Value, op: FloatOp, lhs: &Operand, rhs: &Operand, ty: IrType) => emit_float_binop_impl;
        fn emit_float_binop_impl(&mut self, mnemonic: &str, ty: IrType) => emit_float_binop_impl_impl;
        // i128 ops
        fn emit_sign_extend_acc_high(&mut self) => emit_sign_extend_acc_high_impl;
        fn emit_zero_acc_high(&mut self) => emit_zero_acc_high_impl;
        fn emit_load_acc_pair(&mut self, op: &Operand) => emit_load_acc_pair_impl;
        fn emit_store_acc_pair(&mut self, dest: &Value) => emit_store_acc_pair_impl;
        fn emit_store_pair_to_slot(&mut self, slot: StackSlot) => emit_store_pair_to_slot_impl;
        fn emit_load_pair_from_slot(&mut self, slot: StackSlot) => emit_load_pair_from_slot_impl;
        fn emit_save_acc_pair(&mut self) => emit_save_acc_pair_impl;
        fn emit_store_pair_indirect(&mut self) => emit_store_pair_indirect_impl;
        fn emit_load_pair_indirect(&mut self) => emit_load_pair_indirect_impl;
        fn emit_i128_neg(&mut self) => emit_i128_neg_impl;
        fn emit_i128_not(&mut self) => emit_i128_not_impl;
        fn emit_i128_prep_binop(&mut self, lhs: &Operand, rhs: &Operand) => emit_i128_prep_binop_impl;
        fn emit_i128_add(&mut self) => emit_i128_add_impl;
        fn emit_i128_sub(&mut self) => emit_i128_sub_impl;
        fn emit_i128_mul(&mut self) => emit_i128_mul_impl;
        fn emit_i128_and(&mut self) => emit_i128_and_impl;
        fn emit_i128_or(&mut self) => emit_i128_or_impl;
        fn emit_i128_xor(&mut self) => emit_i128_xor_impl;
        fn emit_i128_shl(&mut self) => emit_i128_shl_impl;
        fn emit_i128_lshr(&mut self) => emit_i128_lshr_impl;
        fn emit_i128_ashr(&mut self) => emit_i128_ashr_impl;
        fn emit_i128_prep_shift_lhs(&mut self, lhs: &Operand) => emit_i128_prep_shift_lhs_impl;
        fn emit_i128_shl_const(&mut self, amount: u32) => emit_i128_shl_const_impl;
        fn emit_i128_lshr_const(&mut self, amount: u32) => emit_i128_lshr_const_impl;
        fn emit_i128_ashr_const(&mut self, amount: u32) => emit_i128_ashr_const_impl;
        fn emit_i128_divrem_call(&mut self, func_name: &str, lhs: &Operand, rhs: &Operand) => emit_i128_divrem_call_impl;
        fn emit_i128_store_result(&mut self, dest: &Value) => emit_i128_store_result_impl;
        fn emit_i128_to_float_call(&mut self, src: &Operand, from_signed: bool, to_ty: IrType) => emit_i128_to_float_call_impl;
        fn emit_float_to_i128_call(&mut self, src: &Operand, to_signed: bool, from_ty: IrType) => emit_float_to_i128_call_impl;
        fn emit_i128_cmp_eq(&mut self, is_ne: bool) => emit_i128_cmp_eq_impl;
        fn emit_i128_cmp_ordered(&mut self, op: IrCmpOp) => emit_i128_cmp_ordered_impl;
        fn emit_i128_cmp_store_result(&mut self, dest: &Value) => emit_i128_cmp_store_result_impl;
    }
}
impl Default for X86Codegen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod machinst_resolution_tests {
    use super::super::machinst::{MachInst, MachReg, OpSize};
    use super::*;

    #[test]
    fn vreg_is_unresolved_even_if_main_map_has_xmm_assignment() {
        let inst = MachInst::Neg {
            dst: MachReg::Vreg(7),
            size: OpSize::S64,
        };
        let mut assignments = FxHashMap::default();
        assignments.insert(7, PhysReg(20));
        assert!(has_unresolvable_vreg(&inst, &assignments));
    }

    #[test]
    fn physical_unary_register_is_resolved() {
        let inst = MachInst::Not {
            dst: MachReg::Phys(PhysReg(1)),
            size: OpSize::S64,
        };
        assert!(!has_unresolvable_vreg(&inst, &FxHashMap::default()));
    }
}
