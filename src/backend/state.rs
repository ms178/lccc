//! Shared codegen state, slot addressing types, and register value cache.
//!
//! All four backends use the same `CodegenState` to track stack slot assignments,
//! alloca metadata, label generation, and the register value cache during code generation.
//! The `SlotAddr` enum captures the 3-way addressing pattern (over-aligned alloca /
//! direct alloca / indirect) that repeats across store, load, GEP, and memcpy emission.
//!
//! The `RegCache` tracks which IR values are currently known to be in registers,
//! enabling backends to skip redundant stack loads. This is the foundation for
//! eventually replacing the pure stack-slot model with a register allocator.

use super::common::AsmOutput;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{BlockId, IrCmpOp, Operand};

/// Stack slot location for a value. Interpretation varies by arch:
/// - x86: negative offset from %rbp
/// - ARM: positive offset from sp
/// - RISC-V: negative offset from s0
#[derive(Debug, Clone, Copy)]
pub struct StackSlot(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitLocation {
    Reg(crate::backend::regalloc::PhysReg),
    Accumulator,
}

/// Register cache entry: tracks which IR value is known to be in a register.
/// The `is_alloca` flag distinguishes whether the register holds the alloca's
/// address (leaq/adr) or the value loaded from the stack slot (movq/ldr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegCacheEntry {
    pub value_id: u32,
    pub is_alloca: bool,
}

/// Register value cache. Tracks which IR values are currently in the accumulator
/// and secondary registers, avoiding redundant stack loads.
///
/// The cache is conservative: it is invalidated on any operation that might clobber
/// a register (calls, inline asm, complex operations that use scratch registers).
/// This is safe because a stale entry would just cause a redundant load (the same
/// behavior as before the cache existed), while a missing invalidation could cause
/// incorrect code by skipping a needed load.
///
/// Architecture mapping:
/// - x86:    acc = %rax,  sec = %rcx
/// - ARM64:  acc = x0,    sec = x1
/// - RISC-V: acc = t0,    sec = t1
#[derive(Debug, Default)]
pub struct RegCache {
    /// Which value is currently in the primary accumulator register (%rax on x86).
    pub acc: Option<RegCacheEntry>,
    /// Which value is currently in the secondary register (%rcx on x86).
    /// Tracked so that operand_to_rax can use `movq %rcx, %rax` instead of
    /// a stack reload when the value was recently loaded to %rcx.
    pub sec: Option<RegCacheEntry>,
}

impl RegCache {
    /// Record that the accumulator now holds the given value.
    #[inline]
    pub fn set_acc(&mut self, value_id: u32, is_alloca: bool) {
        self.acc = Some(RegCacheEntry {
            value_id,
            is_alloca,
        });
    }

    /// Record that the secondary register now holds the given value.
    #[inline]
    pub fn set_sec(&mut self, value_id: u32, is_alloca: bool) {
        self.sec = Some(RegCacheEntry {
            value_id,
            is_alloca,
        });
    }

    /// Check if the accumulator holds the given value (with matching alloca status).
    #[inline]
    pub fn acc_has(&self, value_id: u32, is_alloca: bool) -> bool {
        self.acc
            == Some(RegCacheEntry {
                value_id,
                is_alloca,
            })
    }

    /// Check if the secondary register holds the given value.
    #[inline]
    pub fn sec_has(&self, value_id: u32, is_alloca: bool) -> bool {
        self.sec
            == Some(RegCacheEntry {
                value_id,
                is_alloca,
            })
    }

    /// Invalidate the accumulator cache.
    #[inline]
    pub fn invalidate_acc(&mut self) {
        self.acc = None;
    }

    /// Invalidate the secondary register cache.
    #[inline]
    pub fn invalidate_sec(&mut self) {
        self.sec = None;
    }

    /// Invalidate all cached register values. Called on operations that may
    /// clobber any register (calls, inline asm, etc.).
    #[inline]
    pub fn invalidate_all(&mut self) {
        self.acc = None;
        self.sec = None;
    }
}

/// Shared codegen state, used by all backends.
pub struct CodegenState {
    pub out: AsmOutput,
    /// Set from CodegenOptions for -O0 non-SSA correctness.
    pub disable_regalloc: bool,
    pub stack_offset: i64,
    pub value_locations: FxHashMap<u32, StackSlot>,
    /// Emergency spill slot offset, allocated downward from the bottom of the frame.
    /// Used when store_rax_to encounters a value with no register or stack slot.
    /// Values that are allocas (their stack slot IS the data, not a pointer to data).
    pub alloca_values: FxHashSet<u32>,
    /// Direct allocas declared volatile in the source.  Their memory accesses
    /// are observable and must be quarantined from text peephole rewrites.
    pub volatile_alloca_values: FxHashSet<u32>,
    /// Type associated with each alloca (for type-aware loads/stores).
    pub alloca_types: FxHashMap<u32, IrType>,
    /// Alloca values that need runtime alignment > 16 bytes.
    pub alloca_alignments: FxHashMap<u32, usize>,
    /// Values that are 128-bit integers (need 16-byte copy).
    pub i128_values: FxHashSet<u32>,
    /// Values that are 64-bit types on 32-bit targets (F64, I64/U64).
    /// These values don't fit in a single 32-bit GPR and need special
    /// copy handling (e.g., x87 fldl/fstpl for F64, two-word copy for I64).
    /// Only populated on i686; empty on 64-bit targets.
    pub wide_values: FxHashSet<u32>,
    /// Vector values (produced by Vec* intrinsics like VecZeroF64x4, VecLoadF64x4, etc.).
    /// These should be addressed via leaq (to get stack slot address) instead of movq
    /// (which would load scalar bytes), because they hold 128/256-bit vector data.
    pub vector_values: FxHashSet<u32>,
    /// 128-bit (SSE-class) vector values. Same addressing rules as
    /// `vector_values`, but their home slots are 16 bytes wide rather than 32.
    /// Populated together with `vector_values` by the stack-layout pre-scan for
    /// every user-level SSE/AVX intrinsic result (see
    /// `IntrinsicOp::vector_result_width`).
    pub vector128_values: FxHashSet<u32>,
    /// Vector-register peephole: the stack slot offset that the last
    /// `avx_store_dest`/`sse_store_dest` wrote from %ymm0/%xmm0, and whether
    /// that register still provably holds the value. When the very next vector
    /// operand load targets the same slot, the load is skipped and the register
    /// is reused directly (kills the store→reload round-trip that the
    /// slot-based vector codegen otherwise emits for every intrinsic result).
    /// Cleared conservatively at every call/asm/atomic/memcpy emission (all of
    /// which may clobber the vector register) and on any actual vector load.
    pub vec_last_store_slot: Option<i64>,
    /// The IR value that was stored to `vec_last_store_slot`. The peephole is
    /// only sound when the subsequent load reads the SAME value: stack slots
    /// are coalesced/reused across different values, so a slot match alone is
    /// not enough (a store of A followed by a load of B from the same slot
    /// must NOT reuse the register).
    pub vec_last_store_val: Option<u32>,
    pub vec_last_store_reg: bool,
    /// Which vector register provably holds `vec_last_store_val` (always
    /// "ymm0" for the AVX path today; kept as a name so the loaders can rename
    /// reg-to-reg without a memory round trip).
    pub vec_last_store_reg_name: Option<&'static str>,
    /// For 128-bit register peephole (xmm0 variant), same as above.
    pub sse_last_store_slot: Option<i64>,
    pub sse_last_store_val: Option<u32>,
    pub sse_last_store_reg: bool,
    /// Which xmm register provably holds `sse_last_store_val`. The pblendvb
    /// handler stores its result from %xmm2; generalizing the tracking to all
    /// source registers (not just xmm0) makes the reload-skip sound for those
    /// results too (the "pblendvb xmm2" fix).
    pub sse_last_store_reg_name: Option<&'static str>,
    /// Values whose vector result store may be DEFERRED (skipped entirely):
    /// computed by the v5 IR analysis (`compute_vector_defer_values`) — the
    /// value's single use is as args[0]/args[1] of the immediately-following
    /// intrinsic, so the register it was computed in is the only home it ever
    /// needs. When the store is skipped the value stays in the register and
    /// the consumer's load becomes a no-op (accumulator renaming).
    pub vector_defer_values: FxHashSet<u32>,
    /// VLFOLD: 256-bit `VecLoad*` results whose single use is the adjacent
    /// two-operand VEX arithmetic intrinsic (`compute_vector_memfold_values`).
    /// The load is not emitted; the consumer folds its source memory operand.
    pub vector_memfold_values: FxHashSet<u32>,
    /// The elided load awaiting its consumer: (value id, memory operand,
    /// load mnemonic for materialisation). Consumed by `emit_avx_binary_256`
    /// as a memory operand, or by `avx_load_arg_to` as a real load; the
    /// safety net in `emit_intrinsic_impl` materialises it before any other
    /// intrinsic.
    pub pending_vec_memfold: Option<(u32, String, &'static str)>,
    /// Lazy-flush half of the deferred-store mechanism: a skipped vector
    /// result store is kept PENDING here (value id, holding register, 256-bit
    /// flag) instead of being dropped. If the consumer really receives the
    /// value through the last-store cache, the entry is silently discarded
    /// (pure win: no store ever emitted). Otherwise the store is flushed just
    /// before anything clobbers the holding register, reads the slot, or
    /// leaves the block — so a deferred store can never be observed missing.
    /// This makes deferred stores sound by construction, independent of the
    /// IR analysis' adjacency precision.
    pub pending_vec_store: Option<(u32, &'static str, bool)>,
    /// i686 x87 top-of-stack cache (the FP twin of `pending_vec_store`).
    /// `x87_pending == Some(v)` means: st(0) holds a copy of the F64 value
    /// `v`, its slot was already written by a non-popping `fstl`, and no
    /// other x87 instruction has executed since. `emit_float_binop` sets it
    /// so the next consumer of `v` can dup the register copy (`fld %st(0)`)
    /// or consume it in place (non-popping `fsub %st(1), %st` forms)
    /// instead of reloading from the slot. Flushed (`fstp %st(0)`) by the
    /// central hook in `emit`/`emit_fmt` before any other x87-mutating or
    /// label line, at block labels, calls, inline asm, and returns — so the
    /// x87 stack depth stays bounded by construction and the slot copy is
    /// always correct for ordinary memory readers.
    pub x87_pending: Option<u32>,
    /// F64 binop results whose single use is the operand of the
    /// immediately-following F64 binop (`compute_x87_defer_values`). Only
    /// these may leave their result on the x87 stack with a non-popping
    /// `fstl`, because the adjacency guarantee means no other instruction
    /// can observe or clobber the window between store and consumption.
    pub x87_defer_values: FxHashSet<u32>,
    /// Suppresses the central x87-pending flush while `emit_float_binop`
    /// emits its own controlled x87 sequences (dup / in-place arith / fstl).
    /// Never set across an early-return boundary without being reset.
    pub x87_suppress: bool,
    /// Scalar FP value already resident in the SysV return register xmm0.
    /// Set only by exact direct-result intrinsics immediately before Return.
    pub direct_fp_result: Option<u32>,
    /// CCC_ENABLE_VECREG: value id -> physical XMM register name that currently
    /// provably holds the vector data (128-bit vector register allocation).
    pub vec_live_regs: FxHashMap<u32, &'static str>,
    /// True once this function emitted any 256/512-bit vector instruction.
    /// Dirty upper YMM halves trigger the AVX-SSE transition penalty in
    /// callers that use legacy SSE encodings; the epilogue emits
    /// `vzeroupper` before `ret` when this is set (IS-20).
    pub dirty_upper_ymm: bool,
    /// True once any function emitted a nested-function trampoline. The
    /// stack must be executable for trampolines to work; the output's
    /// .note.GNU-stack section is emitted with the "x" flag.
    pub requires_executable_stack: bool,
    /// Fully-formed `.section .data.rel.ro` template/slot blocks for i686
    /// nested-function trampolines. Collected while functions emit code and
    /// flushed once at the end of the module (AFTER the last text byte) —
    /// switching sections in the middle of a function body interleaves
    /// data directives into the peephole/assembler stream and corrupted
    /// instruction/label mapping at -O1 (see 20000822-1).
    pub trampoline_data_blocks: Vec<String>,
    /// Counter for generating unique labels (e.g., memcpy loops).
    label_counter: u32,
    /// Whether any position-independent code generation is enabled. Backends
    /// that do not distinguish PIE from full PIC retain this conservative bit.
    pub pic_mode: bool,
    /// Position-independent executable mode. x86-64 may address ordinary
    /// executable data directly; full PIC and weak extern data still use GOT.
    pub pie_mode: bool,
    /// Set of symbol names that are locally defined (not extern) and have internal
    /// linkage (static) — these can use direct addressing even in PIC mode.
    pub local_symbols: FxHashSet<String>,
    /// Extern function linker names. Their addresses cannot use ELF data copy
    /// relocations, so default-visibility function pointers remain GOT-based
    /// in PIE even though ordinary extern data addresses may be direct.
    pub extern_function_symbols: FxHashSet<String>,
    /// Compare-replay records: Cmp instructions whose boolean result is
    /// consumed by exactly one Select or CondBranch that is NOT adjacent to
    /// the Cmp (so pending-flag fusion cannot fire — an intervening ALU op
    /// like `sub` clobbers the flags). The Cmp emitter skips setcc/movzbl;
    /// the consumer re-emits `cmp` from the recorded operands and uses
    /// `cmovcc`/`jcc` directly. Recorded per-function, keyed by Cmp dest.
    pub cmp_replay: FxHashMap<u32, (IrCmpOp, Operand, Operand, IrType)>,
    /// Symbols defined by numeric `.set sym, <number>` directives in top-level
    /// asm (glibc-style absolute markers). Their address is a link-time
    /// constant: emitted as `movq $sym`, never via GOT.
    pub absolute_symbols: FxHashSet<String>,
    /// Set of symbol names that are thread-local (_Thread_local / __thread).
    /// These require TLS-specific access patterns (e.g., %fs:x@TPOFF on x86-64).
    pub tls_symbols: FxHashSet<String>,
    /// Whether the current function contains DynAlloca instructions.
    /// When true, the epilogue must restore SP from the frame pointer instead of
    /// adding back the compile-time frame size.
    pub has_dyn_alloca: bool,
    /// Whether to omit the frame pointer for this function.
    /// When true, %rbp is not used for stack frame addressing and can be
    /// used as a general-purpose register. Stack slots use %rsp-relative addressing.
    pub omit_frame_pointer: bool,
    /// The user's `-fomit-frame-pointer` / `-fno-omit-frame-pointer` request.
    /// Module-wide: NOT cleared by `reset_for_function` (unlike
    /// `omit_frame_pointer`, which is derived per function from this flag plus
    /// the function's own constraints). Without this the CLI flag was dropped
    /// entirely and `-fno-omit-frame-pointer` silently did nothing.
    pub fpo_requested: bool,
    /// Total frame size (for RSP-relative addressing when omit_frame_pointer is true).
    /// Set after calculate_stack_space completes.
    pub frame_size: i64,
    /// Register value cache: tracks which IR values are in the accumulator and
    /// secondary registers to skip redundant loads.
    pub reg_cache: RegCache,
    /// GEP decomposition: maps GEP result value ID → (base_value_id, offset_value_id).
    /// Populated during instruction generation for GEPs with variable offsets.
    /// Used by the FMA intrinsic codegen to detect SIB addressing opportunities.
    pub gep_base_offset: FxHashMap<u32, (u32, u32)>,
    /// Set of GEP result value IDs whose instructions were folded into Load/Store
    /// and skipped during codegen. These values are never emitted — their registers
    /// are uninitialized. MachInst must not use these as pointers.
    pub folded_gep_values: FxHashSet<u32>,
    /// Values proven never-materialized before slot assignment (foldable
    /// GlobalAddr on i686 non-PIC): no register, no slot.
    pub never_materialized_values: FxHashSet<u32>,
    /// GlobalAddr values the prologue PROMOTED out of the never-materialized
    /// set (PF-07: PIC indexed-symbol bases defined at a shallower loop depth
    /// than their consumers). The instruction-generation dead-address scan
    /// must NOT skip these: they carry a register home that
    /// `emit_global_addr` materializes once and the SIB consumers read.
    pub promoted_global_addr_homes: FxHashSet<u32>,
    /// Name of the function currently being code-generated (diagnostics).
    pub current_func_name: String,
    /// Total use count for each value ID across the entire function.
    /// Used by MachInst flush to determine if a buffer-defined vreg is live-out.
    pub value_use_counts: Vec<u32>,
    /// Use counts within the current block (set before block codegen).
    /// Compared with value_use_counts to detect live-out values.
    pub block_use_counts: FxHashMap<u32, u32>,
    /// Loads whose single use is an adjacent `Cmp { Eq|Ne, rhs = imm }` and
    /// that are therefore folded into a memory compare (`cmpb/cmpw/cmpl $imm,
    /// (mem)`) by the backend. Keyed by the Load's dest value id; the value
    /// maps to (pointer value id, loaded type, compare immediate) so the Cmp
    /// can re-stage the address and emit the immediate. Cleared per block.
    /// Only the i686 backend sets `supports_load_cmp_mem_fold`.
    pub pending_load_cmp: FxHashMap<u32, (u32, IrType, i64)>,
    /// Whether to replace `ret` with `jmp __x86_return_thunk` (-mfunction-return=thunk-extern).
    /// Used by the Linux kernel for Spectre v2 (retbleed) mitigation.
    pub function_return_thunk: bool,
    /// Whether to replace indirect calls/jumps with retpoline thunks (-mindirect-branch=thunk-extern).
    /// Used by the Linux kernel for Spectre v2 (retpoline) mitigation.
    pub indirect_branch_thunk: bool,
    /// -mindirect-branch=thunk-inline: emit the full retpoline sequence at
    /// every indirect call/jump site instead of calling an external
    /// __x86_indirect_thunk_REG. The kernel vDSO needs this: it is userspace
    /// code that cannot reference kernel-internal thunk symbols.
    pub indirect_branch_thunk_inline: bool,
    /// Is the call currently being emitted a call to a VARIADIC callee?
    ///
    /// Set by the generic `emit_call` before argument setup. SysV AMD64 3.5.7
    /// only requires %al to carry the number of live SSE argument registers
    /// when the callee is variadic; for a prototyped non-variadic callee the
    /// `xorl %eax,%eax` is pure waste. GCC omits it there.
    pub call_is_variadic: bool,
    /// Patchable function entry: (total_nops, nops_before_entry).
    /// When set, emits NOP padding around function entry points and records
    /// them in __patchable_function_entries for runtime patching (ftrace).
    pub patchable_function_entry: Option<(u32, u32)>,
    /// Function-entry mcount instrumentation (-pg / -mfentry / -mrecord-mcount /
    /// -mnop-mcount). When set, emits a 5-byte call (or NOP) at function entry,
    /// optionally recorded in __mcount_loc for the runtime patcher.
    pub mcount: Option<crate::backend::McountInstrumentation>,
    /// Set by generate_function for classic (non-fentry) mcount: the `call
    /// mcount` must be emitted AFTER the frame is established (push %rbp; mov
    /// %rsp, %rbp) — GCC measures to the same shape and the classic mcount ABI
    /// reads the parent PC through the frame. Holds the site label when the
    /// site is recorded in __mcount_loc. The backend prologue consumes
    /// (take()s) the label right after frame setup.
    pub pending_classic_mcount_label: Option<String>,
    /// One-shot warning latch: -pg requested on a target whose backend does
    /// not yet emit mcount sites (honest degradation, never silent).
    pub mcount_unsupported_warned: bool,
    /// Whether to emit endbr64 at function entry points (-fcf-protection=branch).
    pub cf_protection_branch: bool,
    /// Current program point during codegen (incremented per instruction).
    /// Used by Phase 2b selective caller-save save/restore.
    pub current_program_point: u32,
    /// Maps an F128 value ID to the memory location it was loaded from.
    /// This enables full-precision F128 operations (casts, comparisons, stores)
    /// by reloading directly from the original memory location instead of using
    /// a lossy f64 intermediate.
    ///
    /// The tuple is `(source_value_id, byte_offset, is_indirect)`:
    /// - `source_value_id`: The alloca or pointer value whose slot contains the data.
    /// - `byte_offset`: Offset from the source slot's base address.
    /// - `is_indirect`: If true, the source slot holds a pointer that must be
    ///   dereferenced. If false, the F128 data is directly at the slot.
    ///
    /// x86 uses this for x87 `fldt` from original memory (typically offset=0, is_indirect=false).
    /// ARM/RISC-V use this for IEEE binary128 soft-float operations with full 16-byte precision.
    pub f128_load_sources: FxHashMap<u32, (u32, i64, bool)>,
    /// Values whose 16-byte slots contain full x87 80-bit data (via fstpt),
    /// not a pointer. These are F128 call results or other values where
    /// full precision was preserved directly in the slot.
    pub f128_direct_slots: FxHashSet<u32>,
    /// The current text section name for this function. Defaults to ".text" but
    /// may be a custom section (e.g., ".init.text") for functions with
    /// __attribute__((section("..."))). Used to restore the correct section
    /// after emitting data (e.g., jump tables) in other sections.
    pub current_text_section: String,
    /// Whether to use the kernel code model (-mcmodel=kernel). All symbols
    /// are assumed to be in the negative 2GB of the virtual address space.
    /// Uses absolute sign-extended 32-bit addressing (movq $symbol) for
    /// global address references, producing R_X86_64_32S relocations.
    pub code_model_kernel: bool,
    /// Whether to disable jump table emission for switch statements (-fno-jump-tables).
    pub no_jump_tables: bool,
    /// Set of symbol names declared as weak extern (e.g., `extern __weak`).
    /// On AArch64, these need GOT-indirect addressing because the linker
    /// rejects R_AARCH64_ADR_PREL_PG_HI21 against symbols that may bind externally.
    pub weak_extern_symbols: FxHashSet<String>,
    /// SSA values that use 4-byte (32-bit) stack slots instead of the default 8-byte.
    /// On 64-bit targets, I32/U32/F32 and smaller types can use 4-byte slots,
    /// reducing stack frame sizes by ~40%. Store/load paths check this set to
    /// emit 4-byte instructions (movl, sw/lw, str/ldr w-reg) instead of 8-byte.
    pub small_slot_values: FxHashSet<u32>,
    /// Explicit non-stack homes. Register and accumulator residency share one
    /// location contract rather than parallel convention sets.
    pub explicit_locations: FxHashMap<u32, ExplicitLocation>,
    /// Allocator-issued accumulator homes for the current function.
    pub ra_accumulator_values: FxHashSet<u32>,
    /// Values that are promoted InlineAsm output results. Like allocas, their
    /// stack slot holds the value directly (not a pointer). The asm emitter
    /// stores the output register to this slot after the asm, and subsequent
    /// uses load the value from it.
    pub asm_output_values: FxHashSet<u32>,
    /// Values whose stack slots must not be reused for other values.
    /// This includes DynAlloca results (which hold pointers to dynamically
    /// allocated stack space) and other vectorization-critical values.
    /// The backend's slot allocation must preserve these values' slots for
    /// their entire live range to prevent corruption.
    pub protected_slot_values: FxHashSet<u32>,
    /// Whether to emit .file/.loc debug directives for source-level debugging.
    pub debug_info: bool,
    /// Pre-computed parameter classifications for the current function.
    /// Populated by `emit_store_params` so that `emit_param_ref` can access them.
    pub param_classes: Vec<crate::backend::call_abi::ParamClass>,
    /// Number of function parameters (for ParamRef bounds checking).
    pub num_params: usize,
    /// Whether the current function is variadic (for ParamRef ABI handling).
    pub func_is_variadic: bool,
    /// Stack slots for parameter allocas, indexed by param_idx.
    /// Populated by `emit_store_params` so that `emit_param_ref` can load the
    /// parameter value from its alloca slot (where emit_store_params saved it)
    /// instead of reading from ABI registers that may have been clobbered.
    pub param_alloca_slots: Vec<Option<(StackSlot, IrType)>>,
    /// Set of param indices whose values have been pre-stored directly to a
    /// callee-saved register during `emit_store_params`. `emit_param_ref` can
    /// skip the alloca load for these and just emit a no-op (the value is
    /// already in the register-allocated destination).
    pub param_pre_stored: FxHashSet<usize>,
    /// Whether the current function returns a struct via hidden pointer (sret).
    /// On i386 SysV ABI, such functions must use `ret $4` to pop the hidden
    /// pointer argument from the caller's stack.
    pub uses_sret: bool,
    /// Whether to place each function in its own section (-ffunction-sections).
    pub function_sections: bool,
    /// Whether to place each data object in its own section (-fdata-sections).
    pub data_sections: bool,
    /// Whether any 64-bit division/modulo runtime helpers (__divdi3, __udivdi3,
    /// __moddi3, __umoddi3) were referenced during code generation.
    /// When true, the i686 backend emits weak implementations of these functions
    /// so that standalone builds (without libgcc) can link successfully.
    pub needs_divdi3_helpers: bool,
    /// Whether to emit CFI directives (.cfi_startproc, .cfi_endproc, etc.)
    /// for generating .eh_frame unwind tables. Enabled by default (like GCC).
    pub emit_cfi: bool,

    /// Floating-point constant pool: maps bit pattern → label name.
    /// FP constants are emitted as .rodata entries and loaded via
    /// `movsd .LCFPxx(%rip), %xmm` instead of `movabsq + movq`.
    pub fp_const_pool: FxHashMap<u64, String>,
    /// Label emitted immediately after the current block, if any. Used to
    /// elide unconditional jumps on the hot fall-through path.
    pub next_block_label: Option<BlockId>,
}

impl CodegenState {
    pub fn new() -> Self {
        Self {
            out: AsmOutput::new(),
            disable_regalloc: false,
            stack_offset: 0,
            value_locations: FxHashMap::default(),
            alloca_values: FxHashSet::default(),
            volatile_alloca_values: FxHashSet::default(),
            alloca_types: FxHashMap::default(),
            alloca_alignments: FxHashMap::default(),
            i128_values: FxHashSet::default(),
            wide_values: FxHashSet::default(),
            vector_values: FxHashSet::default(),
            vector128_values: FxHashSet::default(),
            vec_last_store_slot: None,
            vec_last_store_val: None,
            vec_last_store_reg: false,
            vec_last_store_reg_name: None,
            sse_last_store_slot: None,
            sse_last_store_val: None,
            sse_last_store_reg: false,
            sse_last_store_reg_name: None,
            vector_defer_values: FxHashSet::default(),
            vector_memfold_values: FxHashSet::default(),
            pending_vec_memfold: None,
            pending_vec_store: None,
            x87_pending: None,
            x87_defer_values: FxHashSet::default(),
            x87_suppress: false,
            direct_fp_result: None,
            vec_live_regs: FxHashMap::default(),
            dirty_upper_ymm: false,
            requires_executable_stack: false,
            trampoline_data_blocks: Vec::new(),
            label_counter: 0,
            pic_mode: false,
            pie_mode: false,
            local_symbols: FxHashSet::default(),
            extern_function_symbols: FxHashSet::default(),
            cmp_replay: FxHashMap::default(),
            absolute_symbols: FxHashSet::default(),
            tls_symbols: FxHashSet::default(),
            has_dyn_alloca: false,
            omit_frame_pointer: false,
            fpo_requested: true, // -fomit-frame-pointer is the default at -O1+
            frame_size: 0,
            reg_cache: RegCache::default(),
            gep_base_offset: FxHashMap::default(),
            folded_gep_values: FxHashSet::default(),
            never_materialized_values: FxHashSet::default(),
            promoted_global_addr_homes: FxHashSet::default(),
            current_func_name: String::new(),
            value_use_counts: Vec::new(),
            block_use_counts: FxHashMap::default(),
            pending_load_cmp: FxHashMap::default(),
            function_return_thunk: false,
            indirect_branch_thunk: false,
            indirect_branch_thunk_inline: false,
            call_is_variadic: false,
            patchable_function_entry: None,
            mcount: None,
            pending_classic_mcount_label: None,
            mcount_unsupported_warned: false,
            cf_protection_branch: false,
            current_program_point: 0,
            f128_load_sources: FxHashMap::default(),
            f128_direct_slots: FxHashSet::default(),
            current_text_section: ".text".to_string(),
            code_model_kernel: false,
            no_jump_tables: false,
            weak_extern_symbols: FxHashSet::default(),
            small_slot_values: FxHashSet::default(),
            explicit_locations: FxHashMap::default(),
            ra_accumulator_values: FxHashSet::default(),
            asm_output_values: FxHashSet::default(),
            protected_slot_values: FxHashSet::default(),
            debug_info: false,
            param_classes: Vec::new(),
            num_params: 0,
            func_is_variadic: false,
            param_alloca_slots: Vec::new(),
            param_pre_stored: FxHashSet::default(),
            uses_sret: false,
            function_sections: false,
            data_sections: false,
            needs_divdi3_helpers: false,
            emit_cfi: true,

            fp_const_pool: FxHashMap::default(),
            next_block_label: None,
        }
    }

    pub fn next_label_id(&mut self) -> u32 {
        let id = self.label_counter;
        self.label_counter += 1;
        id
    }

    /// Mark that a `.section` directive for a NON-TEXT section has just been
    /// emitted (e.g. `__patchable_function_entries`, `__mcount_loc`).
    /// `emit_switch_to_section` compares `current_text_section` against the
    /// requested section to skip no-op switches; without invalidating here,
    /// `current_text_section` would still read as the last text section
    /// (`.text` or a custom `-ffunction-sections` name) and the switch back
    /// would be skipped, leaving the function body in the writable, non-
    /// executable data section — calling the function would then segfault
    /// (reproduced on main: `-fpatchable-function-entry=2,0` placed whole
    /// function bodies into `__patchable_function_entries,"awo"`).
    pub fn invalidate_text_section(&mut self) {
        self.current_text_section.clear();
    }

    /// Generate a fresh label with the given prefix.
    pub fn fresh_label(&mut self, prefix: &str) -> String {
        let id = self.next_label_id();
        format!(".L{}_{}", prefix, id)
    }

    /// Central x87-pending flush gate. Any emitted line that could mutate
    /// the x87 stack (all x87 mnemonics start with `f`) or that begins a
    /// merge point (label lines end with `:`) invalidates the cached
    /// top-of-stack copy first by popping it. The slot copy written by the
    /// non-popping `fstl` remains correct for memory readers, so this only
    /// ever costs one `fstp %st(0)`. Inert on every backend that never sets
    /// `x87_pending` (x86-64/ARM/RISC-V) and while `x87_suppress` is set
    /// (the binop's own controlled sequences).
    fn x87_pre_emit(&mut self, s: &str) {
        if self.x87_pending.is_some() && !self.x87_suppress {
            let is_x87_line = s.len() >= 5 && s.starts_with("    f");
            if is_x87_line || s.ends_with(':') {
                self.x87_pending = None;
                self.out.emit("    fstp %st(0)");
            }
        }
    }

    pub fn emit(&mut self, s: &str) {
        self.x87_pre_emit(s);
        self.out.emit(s);
    }

    /// Get or create a .rodata label for a floating-point constant.
    /// Returns the label name (e.g., ".LCFP_3"). Constants are deduped
    /// so the same bit pattern always returns the same label.
    pub fn get_fp_const_label(&mut self, bits: u64) -> String {
        if let Some(label) = self.fp_const_pool.get(&bits) {
            return label.clone();
        }
        let label = format!(".LCFP_{}", self.label_counter);
        self.label_counter += 1;
        self.fp_const_pool.insert(bits, label.clone());
        label
    }

    /// Emit all accumulated FP constants as a .rodata section.
    /// Called once at the end of module codegen.
    ///
    /// Every constant gets a 16-byte-aligned, 16-byte slot (the 8-byte value
    /// followed by an 8-byte zero pad). The scalar readers (movsd/movss) only
    /// read the first 8/4 bytes, but the Fabs emitter loads these constants
    /// with `andpd`/`andps`, whose m128 memory operand must be 16-byte
    /// aligned — an 8-byte-aligned slot raises #GP whenever the constant
    /// lands on an 8-mod-16 offset (reproduced: two FP functions in one TU,
    /// F64 fabs + F32 fabs, SIGSEGV at the andpd). The zero pad also makes
    /// the upper lane of the andpd mask zero, which is the correct identity
    /// for scalar-typed lanes.
    pub fn emit_fp_const_pool(&mut self) {
        if self.fp_const_pool.is_empty() {
            return;
        }
        self.out.emit(".section .rodata");
        // Sort by label name for deterministic output
        let mut entries: Vec<_> = self.fp_const_pool.iter().collect();
        entries.sort_by_key(|(_, label)| (*label).clone());
        for (bits, label) in entries {
            // .p2align, NOT .align: on AArch64/RISC-V gas `.align N` means
            // 2^N bytes, so `.align 16` silently requested 64KiB alignment
            // per constant. That bloated .rodata by ~64KiB per entry and
            // pushed the pool out of the AArch64 ldr-literal ±1MB range
            // (R_AARCH64_LD_PREL_LO19 truncation at static link). .p2align 4
            // is 16 bytes on every gas target.
            self.out.emit(".p2align 4");
            self.out.emit_fmt(format_args!("{}:", label));
            self.out
                .emit_fmt(format_args!("    .quad {}", *bits as i64));
            self.out.emit("    .quad 0");
        }
    }

    /// Emit formatted assembly directly (no temporary String allocation).
    #[inline]
    pub fn emit_fmt(&mut self, args: std::fmt::Arguments<'_>) {
        // Only pay for string formatting when a pending x87 copy is live
        // (the flush gate needs the fully-formatted line to classify it).
        if self.x87_pending.is_some() && !self.x87_suppress {
            let s = std::fmt::format(args);
            self.x87_pre_emit(&s);
            self.out.emit(&s);
        } else {
            self.out.emit_fmt(args);
        }
    }

    /// Emit a visibility directive (.hidden, .protected, .internal) if the symbol
    /// has non-default visibility. No-op if `visibility` is None or "default".
    pub fn emit_visibility(&mut self, name: &str, visibility: &Option<String>) {
        if let Some(ref vis) = visibility {
            match vis.as_str() {
                "hidden" => self.emit_fmt(format_args!(".hidden {}", name)),
                "protected" => self.emit_fmt(format_args!(".protected {}", name)),
                "internal" => self.emit_fmt(format_args!(".internal {}", name)),
                _ => {} // "default" or unknown: no directive needed
            }
        }
    }

    /// Emit linkage directives (.globl or .weak) for a non-static symbol.
    /// No-op if `is_static` is true.
    pub fn emit_linkage(&mut self, name: &str, is_static: bool, is_weak: bool) {
        if !is_static {
            if is_weak {
                self.emit_fmt(format_args!(".weak {}", name));
            } else {
                self.emit_fmt(format_args!(".globl {}", name));
            }
        }
    }

    pub fn reset_for_function(&mut self) {
        self.stack_offset = 0;
        self.value_locations.clear();
        self.alloca_values.clear();
        self.volatile_alloca_values.clear();
        self.alloca_types.clear();
        self.alloca_alignments.clear();
        self.i128_values.clear();
        self.wide_values.clear();
        self.never_materialized_values.clear();
        self.promoted_global_addr_homes.clear();
        self.has_dyn_alloca = false;
        self.omit_frame_pointer = false;
        self.pending_classic_mcount_label = None;
        self.frame_size = 0;
        self.reg_cache.invalidate_all();
        self.f128_direct_slots.clear();
        self.f128_load_sources.clear();
        self.small_slot_values.clear();
        self.vector_values.clear();
        self.protected_slot_values.clear();
        self.explicit_locations.clear();
        self.ra_accumulator_values.clear();
        self.asm_output_values.clear();
        self.param_pre_stored.clear();
        self.vec_last_store_slot = None;
        self.vec_last_store_val = None;
        self.vec_last_store_reg = false;
        self.vec_last_store_reg_name = None;
        self.sse_last_store_slot = None;
        self.sse_last_store_val = None;
        self.sse_last_store_reg = false;
        self.sse_last_store_reg_name = None;
        // Per-function vector classification sets MUST be cleared here. Value
        // IDs are function-local; leaving these populated leaks classifications
        // from previously compiled functions into the next one (a scalar value
        // in function B colliding with a vector intrinsic result ID from
        // function A is then copied via the vector path, emitting LEA instead
        // of LOAD — adler32-style miscompile: vs2 initialized with the address
        // of its slot).
        self.vector_values.clear();
        self.vector128_values.clear();
        self.protected_slot_values.clear();
        self.vector_defer_values.clear();
        self.vector_memfold_values.clear();
        self.pending_vec_memfold = None;
        self.pending_vec_store = None;
        self.x87_pending = None;
        self.x87_defer_values.clear();
        self.x87_suppress = false;
        self.direct_fp_result = None;
        self.vec_live_regs.clear();
        self.dirty_upper_ymm = false;
        self.uses_sret = false;
        self.next_block_label = None;
        // GEP-fold bookkeeping is function-local too: `folded_gep_values`
        // holds the value IDs of skipped address producers, and value IDs
        // restart at 0 in every function.  Without this clear, IDs leaked
        // from function A made `rematerialize_const_addr` fire spuriously in
        // function B on a colliding ID (emitting a stray lea over B's live
        // accumulator / rematerialising a GEP that was never skipped there) —
        // zlib-ng gen_bitlen corruption once the register-base fold
        // relaxation flooded the set, making collisions deterministic.
        // `gep_base_offset` is the same class (per-function GEP base/index
        // cache keyed by value ID).
        self.folded_gep_values.clear();
        self.gep_base_offset.clear();
    }

    /// Invalidate the vector-register peephole state. Called at every emission
    /// that may clobber %ymm0/%xmm0 or break straight-line guarantees: calls,
    /// inline asm, atomics, memcpy, and block boundaries.
    /// Invalidate scratch-register forwarding without discarding vector values
    /// held in allocator-managed registers. Scalar FP emission can overwrite
    /// xmm0/xmm1 while xmm3+ vector homes remain valid, so a deferred value
    /// pending in the scratch registers must be flushed but the allocator
    /// homes must stay live.
    pub fn invalidate_vec_scratch_peephole(&mut self) {
        self.vec_last_store_slot = None;
        self.vec_last_store_val = None;
        self.vec_last_store_reg = false;
        self.vec_last_store_reg_name = None;
        self.sse_last_store_slot = None;
        self.sse_last_store_val = None;
        self.sse_last_store_reg = false;
        self.sse_last_store_reg_name = None;
        self.direct_fp_result = None;
    }

    /// Invalidate all vector-register peephole state. Called when control flow
    /// or an ABI/asm clobber also makes allocator-managed vector homes unsafe.
    pub fn invalidate_vec_peephole(&mut self) {
        self.invalidate_vec_scratch_peephole();
        self.vec_live_regs.clear();
        // NOTE: dirty_upper_ymm is deliberately NOT cleared here. The vec
        // peephole cache is compiler bookkeeping, but upper-half dirtiness is
        // physical machine state that persists until an explicit vzeroupper
        // (or the epilogue's); invalidating the cache mid-function must not
        // lose the pending epilogue emission.
    }

    /// Get the over-alignment requirement for an alloca (> 16 bytes), or None.
    pub fn alloca_over_align(&self, v: u32) -> Option<usize> {
        self.alloca_alignments.get(&v).copied()
    }

    pub fn is_alloca(&self, v: u32) -> bool {
        self.alloca_values.contains(&v)
    }

    /// Check if a value has a direct stack slot (the slot holds the value itself,
    /// not a pointer to the value). This is true for allocas and promoted InlineAsm
    /// output values.
    pub fn is_direct_slot(&self, v: u32) -> bool {
        self.alloca_values.contains(&v) || self.asm_output_values.contains(&v)
    }

    pub fn get_slot(&self, v: u32) -> Option<StackSlot> {
        self.value_locations.get(&v).copied()
    }

    pub fn is_i128_value(&self, v: u32) -> bool {
        self.i128_values.contains(&v)
    }

    /// Check if a value uses a 4-byte (small) stack slot.
    /// Used by store/load paths to emit 4-byte instructions instead of 8-byte.
    ///
    /// Soundness contract (enforced by the slot allocator):
    ///   1. Small slots are only allocated on targets whose store/load paths
    ///      are width-consistent (x86-64 and i686; see
    ///      set_target_small_slots), and only for values proven no wider than
    ///      four bytes. i686's proof propagates through untyped Copy webs.
    ///   2. Slot sharing is partitioned by exact size class — Tier 3 free
    ///      lists, Tier 2 graph coloring and copy-alias resolution never mix
    ///      4-byte and 8-byte occupants of the same physical slot. A 4-byte
    ///      store therefore can never leave stale bytes that a later 64-bit
    ///      access of the SAME slot could observe.
    ///   3. Every access path honours the width: stores use movl, loads use
    ///      the type-aware movzbl/movzwl/movl/movslq family, and ALU/compare
    ///      memory-operand folding is suppressed whenever it would touch a
    ///      small slot with an 8-byte form.
    /// With those invariants, the old hazard (a 32-bit value sharing a slot
    /// with a pointer, whose upper half a 64-bit reader then observed) is
    /// structurally impossible rather than merely avoided.
    pub fn is_small_slot(&self, v: u32) -> bool {
        self.small_slot_values.contains(&v)
    }

    /// Check if a value is a "wide" type on 32-bit targets (F64, I64, U64).
    /// These need multi-word copy handling instead of the 32-bit accumulator path.
    pub fn is_wide_value(&self, v: u32) -> bool {
        self.wide_values.contains(&v)
    }

    /// Track that `dest_id` was loaded from `source_id` at the given byte offset.
    /// Automatically determines `is_indirect` based on whether the source is an alloca.
    /// Allocas have data directly in the slot; non-allocas hold pointers that must be dereferenced.
    #[inline]
    pub fn track_f128_load(&mut self, dest_id: u32, source_id: u32, offset: i64) {
        let is_indirect = !self.is_alloca(source_id);
        self.f128_load_sources
            .insert(dest_id, (source_id, offset, is_indirect));
    }

    /// Track that `value_id` has full F128 data stored directly in its own slot.
    /// (Used after operations like negation or cast that produce full-precision F128 results.)
    #[inline]
    pub fn track_f128_self(&mut self, value_id: u32) {
        self.f128_load_sources
            .insert(value_id, (value_id, 0, false));
    }

    /// Look up the F128 load source for a value: `(source_id, offset, is_indirect)`.
    #[inline]
    pub fn get_f128_source(&self, value_id: u32) -> Option<(u32, i64, bool)> {
        self.f128_load_sources.get(&value_id).copied()
    }

    /// Propagate F128 source tracking through a Copy: `dest` receives the
    /// exact same source binding as `src` (the copy moves all 16 bytes, so
    /// every consumer of `dest` can read them from the same place `src`'s
    /// consumers would have). Without this, a Copy through an untracked
    /// temporary silently downgrades later uses to the lossy f64-extend
    /// fallback.
    #[inline]
    pub fn track_f128_copy(&mut self, dest_id: u32, src_id: u32) {
        if let Some(entry) = self.f128_load_sources.get(&src_id).copied() {
            self.f128_load_sources.insert(dest_id, entry);
        }
    }

    /// Returns true if the given symbol needs GOT indirection in PIC mode.
    /// A symbol needs GOT if PIC is enabled AND it's not a local (static) symbol.
    /// Local labels (starting with '.') are always PIC-safe via RIP-relative.
    pub fn needs_got(&self, name: &str) -> bool {
        if !self.pic_mode {
            return false;
        }
        if name.starts_with('.') {
            return false;
        }
        !self.local_symbols.contains(name)
    }

    /// Returns true if taking the address of a symbol requires GOT indirection
    /// on x86-64.
    ///
    /// Full PIC keeps default-visibility data interposable and therefore uses
    /// GOTPCREL. PIE is different: definitions in the main executable are not
    /// preemptible, and ordinary extern data can use direct RIP-relative access
    /// plus a copy relocation. Weak externs still need a GOT slot because their
    /// address may resolve to zero. This is the GCC/Clang/ICX executable model;
    /// conflating PIE with PIC added an address load and a long-lived temporary
    /// to every gzip global access.
    pub fn needs_got_for_addr(&self, name: &str) -> bool {
        if self.code_model_kernel || name.starts_with('.') {
            return false;
        }
        if self.local_symbols.contains(name) {
            return false;
        }
        if self.pie_mode {
            // RA-01 kill switch restores the old fully-GOT default so workload
            // A/B runs isolate direct-global rematerialisation exactly.
            if std::env::var_os("CCC_NO_GLOBAL_ADDR_REMAT").is_some() {
                return true;
            }
            return self.weak_extern_symbols.contains(name)
                || self.extern_function_symbols.contains(name);
        }
        self.pic_mode
    }

    /// Returns true if a function call needs PLT indirection in PIC mode.
    pub fn needs_plt(&self, name: &str) -> bool {
        self.needs_got(name)
    }

    /// Returns true if a symbol needs GOT indirection on AArch64.
    ///
    /// In PIC mode (-fPIC/-fpic/-shared), external symbols that may bind
    /// externally must use GOT-indirect ADRP+LDR :got: sequences instead of
    /// direct ADRP+ADD. The system linker rejects R_AARCH64_ADR_PREL_PG_HI21
    /// against symbols that may bind externally in shared objects.
    ///
    /// Weak extern symbols always need GOT indirection regardless of PIC mode,
    /// because they may resolve to zero at runtime.
    pub fn needs_got_aarch64(&self, name: &str) -> bool {
        if name.starts_with('.') {
            return false;
        }
        // Weak extern symbols always need GOT indirection on AArch64
        if self.weak_extern_symbols.contains(name) {
            return true;
        }
        // In PIC mode, external symbols need GOT indirection (same as x86)
        if self.pic_mode {
            return !self.local_symbols.contains(name);
        }
        false
    }
}

/// How a value's effective address is accessed. This captures the 3-way decision
/// (alloca with over-alignment / alloca direct / non-alloca indirect) that repeats
/// across emit_store, emit_load, emit_gep, and emit_memcpy.
#[derive(Debug, Clone, Copy)]
pub enum SlotAddr {
    /// Alloca with alignment > 16: runtime-aligned address must be computed.
    OverAligned(StackSlot, u32),
    /// Normal alloca: slot IS the data, access directly.
    Direct(StackSlot),
    /// Non-alloca: slot holds a pointer that must be loaded first.
    Indirect(StackSlot),
    /// Pointer value already resides in an allocated physical register.
    Reg(crate::backend::regalloc::PhysReg),
}

impl CodegenState {
    #[inline]
    pub fn is_accumulator_location(&self, val_id: u32) -> bool {
        matches!(
            self.explicit_locations.get(&val_id),
            Some(ExplicitLocation::Accumulator)
        )
    }

    /// Classify how to access a value's effective address.
    /// Returns `None` if the value has no assigned stack slot (and isn't register-assigned).
    pub fn resolve_slot_addr(&self, val_id: u32) -> Option<SlotAddr> {
        if let Some(slot) = self.get_slot(val_id) {
            if self.is_alloca(val_id) {
                if self.alloca_over_align(val_id).is_some() {
                    Some(SlotAddr::OverAligned(slot, val_id))
                } else {
                    Some(SlotAddr::Direct(slot))
                }
            } else if self.i128_values.contains(&val_id)
                || self.f128_direct_slots.contains(&val_id)
                || self.vector_values.contains(&val_id)
                || self.vector128_values.contains(&val_id)
            {
                // 16/32-byte payload values (i128/U128/F128 bit patterns, x87
                // bit-op results, and the auto-vectorizer's Vec* values) live
                // DIRECTLY in their slot — the slot is the data, not a pointer
                // to it. Treat as Direct so memcpy, value_ptr_mem_operand, and
                // friends take the slot address instead of loading the first 8
                // payload bytes as a pointer (SEGV). Vec* values are stored
                // with vmovupd/movups straight into the slot by the Vec*
                // emitters; without this they resolved to Indirect and the
                // defer-aware binary emitters fell back to a leaq+indirect
                // load round-trip (v3 reduction-loop codegen).
                Some(SlotAddr::Direct(slot))
            } else {
                Some(SlotAddr::Indirect(slot))
            }
        } else if let Some(ExplicitLocation::Reg(reg)) = self.explicit_locations.get(&val_id) {
            Some(SlotAddr::Reg(*reg))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod slot_addr_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;

    #[test]
    fn register_home_is_explicit_and_exact() {
        let mut state = CodegenState::new();
        state
            .explicit_locations
            .insert(17, ExplicitLocation::Reg(PhysReg(14)));
        assert!(matches!(
            state.resolve_slot_addr(17),
            Some(SlotAddr::Reg(PhysReg(14)))
        ));
        assert!(state.get_slot(17).is_none());
        state
            .explicit_locations
            .insert(18, ExplicitLocation::Accumulator);
        assert!(state.is_accumulator_location(18));
        assert!(state.resolve_slot_addr(18).is_none());
    }
}
