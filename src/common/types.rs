use std::cell::Cell;
use std::rc::Rc;

use crate::common::fx_hash::FxHashMap;

// ── Target data model thread-local ───────────────────────────────────────────
//
// The pointer/long size depends on the target architecture (8 for LP64, 4 for ILP32).
// We use a thread-local because CType::size_ctx() and IrType::size() are called from
// many places that don't have access to a Target parameter. The driver sets this at
// the start of compilation based on the selected target.

thread_local! {
    static TARGET_PTR_SIZE: Cell<usize> = const { Cell::new(8) }; // default: LP64
    /// Whether long double is IEEE binary128 (f128). True for AArch64/RISC-V,
    /// false for x86/i686 where long double is x87 80-bit.
    static TARGET_LONG_DOUBLE_IS_F128: Cell<bool> = const { Cell::new(false) };

    /// ELF e_machine of the current target (EM_X86_64 default). Relocation
    /// number spaces are per ISA; see `target_elf_machine`.
    static TARGET_ELF_MACHINE: Cell<u16> = const { Cell::new(62) };
    /// Whether the current backend supports width-partitioned 4-byte spill
    /// slots. Only the x86-64 backend's store/load paths are fully
    /// width-consistent for small types (see state::is_small_slot); every
    /// other backend keeps the 8-byte fallback until it is audited.
    static TARGET_SMALL_SLOTS: Cell<bool> = const { Cell::new(false) };
    /// GCC's `BIGGEST_ALIGNMENT` in bytes for the current target: the cap
    /// C11 `_Alignof(type)` applies to types that have no *user*
    /// alignment (`min_align_of_type`). x86: 16, 32 under `-mavx`, 64
    /// under `-mavx512f`. See `CType::c11_alignof_ctx`.
    static TARGET_BIGGEST_ALIGNMENT: Cell<usize> = const { Cell::new(16) };
    /// The widest vector register the x86-64 psABI argument classification
    /// may use, in bytes: 16 (SSE), 32 under `-mavx` (`%ymm`), 64 under
    /// `-mavx512f` (`%zmm`). See `target_vector_abi_bytes`.
    static TARGET_VECTOR_ABI_BYTES: Cell<usize> = const { Cell::new(16) };
    /// Widest packed-SIMD register the *lowering* may use for GCC
    /// vector-extension arithmetic (`vector_size` / `ext_vector_type`):
    /// 0 = scalarize every lane, 16 = SSE2 `%xmm`, 32 = AVX2 `%ymm`. See
    /// `set_target_vector_lowering_bytes`.
    static TARGET_VECTOR_LOWERING_BYTES: Cell<usize> = const { Cell::new(0) };
}

/// Publish the widest packed register the frontend may lower a whole
/// vector-extension operation to (`0`, `16` or `32`). Distinct from
/// [`set_target_vector_abi_bytes`]: that one is the psABI *classification*
/// width, which follows the requested ISA only; this one is the code
/// generation ceiling (an AVX2 grant widens the code the backend may emit
/// without changing the ABI). `0` keeps the per-lane scalar lowering — every target
/// without the x86-64 `Vec*` intrinsic families, `-mno-sse`, and `-O0`.
pub fn set_target_vector_lowering_bytes(bytes: usize) {
    debug_assert!(matches!(bytes, 0 | 16 | 32));
    TARGET_VECTOR_LOWERING_BYTES.with(|c| c.set(bytes));
}

/// The widest packed register the vector-extension lowering may use
/// (`0` = scalarize). See [`set_target_vector_lowering_bytes`].
pub fn target_vector_lowering_bytes() -> usize {
    TARGET_VECTOR_LOWERING_BYTES.with(|c| c.get())
}

/// Set the widest vector register the psABI classification may use for a
/// by-value vector argument or return (bytes; 16, 32 or 64). Must be
/// called before any function is lowered.
pub fn set_target_vector_abi_bytes(bytes: usize) {
    debug_assert!(matches!(bytes, 16 | 32 | 64));
    TARGET_VECTOR_ABI_BYTES.with(|c| c.set(bytes));
}

/// The widest vector register the x86-64 psABI argument classification may
/// use, in bytes.
///
/// The psABI classifies a 32-byte `__m256` as SSE + 3×SSEUP and a 64-byte
/// `__m512` as SSE + 7×SSEUP — one `%ymm` / `%zmm` register — "if AVX /
/// AVX-512 is enabled"; otherwise they are MEMORY. GCC (`TARGET_AVX`,
/// `TARGET_AVX512F` in `classify_argument`) and Clang (`AVXLevel` in
/// `X86_64ABIInfo`, `getNativeVectorSizeForAVXABI`) both decide this from
/// the translation unit's *requested* ISA — the same state that defines
/// `__AVX__` / `__AVX512F__` and GCC's `BIGGEST_ALIGNMENT` — not from what
/// the code generator is otherwise allowed to emit. lccc follows that
/// exactly (`Driver::run` publishes it next to `set_target_biggest_alignment`),
/// so a TU built with `-mavx2` interoperates with `gcc -mavx2` and a TU
/// built without `-mavx` keeps the memory convention `gcc` uses without it.
/// Variadic calls are unaffected: the psABI passes `__m256`/`__m512` to a
/// variadic callee in memory regardless (GCC and Clang agree), which
/// `classify_args_core` applies through `real_variadic`.
pub fn target_vector_abi_bytes() -> usize {
    TARGET_VECTOR_ABI_BYTES.with(|c| c.get())
}

/// Set GCC's `BIGGEST_ALIGNMENT` (bytes) for the current target/ISA. Must
/// be called before any `_Alignof` evaluation.
pub fn set_target_biggest_alignment(bytes: usize) {
    debug_assert!(bytes.is_power_of_two());
    TARGET_BIGGEST_ALIGNMENT.with(|c| c.set(bytes));
}

/// GCC's `BIGGEST_ALIGNMENT` (bytes) for the current target/ISA.
pub fn target_biggest_alignment() -> usize {
    TARGET_BIGGEST_ALIGNMENT.with(|c| c.get())
}

/// `TYPE_ALIGN` of a GCC `vector_size(total_size)` type.
///
/// GCC's `layout_type` aligns every vector type naturally — to its full
/// size — "to prevent ABI changes depending on whether or not native
/// vector modes are supported", clamped only by the target hook
/// `TARGET_VECTOR_ALIGNMENT`. On x86 (both -m64 and -m32) the hook is the
/// default (uncapped: `__alignof__` of a 32/64/128/256-byte vector is
/// 32/64/128/256 with or without AVX — GCC 16.2), and that value drives
/// struct layout (`struct { int z; v32 v; }` is 64 bytes with `v` at 32),
/// variable alignment (`.align 32`), and parameter passing (the
/// "ABI for passing parameters with 32-byte alignment has changed in GCC
/// 4.6" note). Only C11 `_Alignof` reports a smaller number, via
/// `min_align_of_type`'s `BIGGEST_ALIGNMENT` cap — see
/// [`CType::c11_alignof_ctx`]. AArch64 overrides the hook to cap at 16
/// (`aarch64_vector_alignment`); ARM and RISC-V keep the historical 16
/// cap here until measured against a reference compiler.
pub fn vector_type_align(total_size: usize) -> usize {
    match target_elf_machine() {
        // EM_X86_64 / EM_386
        62 | 3 => total_size,
        _ => total_size.min(16),
    }
}

/// Set whether the current target allows 4-byte spill slots (x86-64 only).
/// Must be called before stack layout runs.
pub fn set_target_small_slots(enabled: bool) {
    TARGET_SMALL_SLOTS.with(|c| c.set(enabled));
}

/// Whether the current target allows 4-byte spill slots.
pub fn target_small_slots() -> bool {
    TARGET_SMALL_SLOTS.with(|c| c.get())
}

/// Set the target pointer size for the current thread (4 for i686/ILP32, 8 for LP64).
/// Must be called before any type size queries.
pub fn set_target_ptr_size(size: usize) {
    TARGET_PTR_SIZE.with(|c| c.set(size));
}

/// Largest object size this compiler will lay out, in bytes: the *target's*
/// `PTRDIFF_MAX`.
///
/// GCC uses the same bound ("maximum object size" in its diagnostics) and it
/// is the only value that keeps C's object-size arithmetic well defined: an
/// object may be at most `PTRDIFF_MAX` bytes, because pointer subtraction
/// between two pointers into it must be representable in `ptrdiff_t`.  The
/// value therefore depends on the target pointer width, not on the host's:
/// on `-m32` the limit is 2^31-1, and using the 64-bit limit there accepted
/// `char a[4294967296]` -- whose element count then truncated to zero in a
/// 32-bit `usize` and produced a zero-byte `.bss` (GCC rejects it).
///
/// Every byte-size computation must test this limit in `u128` *before*
/// narrowing, because the arithmetic that computes it overflows exactly for
/// the inputs that need rejecting: `int a[4611686018427387904]` is 2^64 bytes,
/// which wraps to 0 in 64-bit arithmetic -- the compiler used to accept that
/// and emit a zero-byte `.bss` (see `size_ctx`, which saturates instead).
#[inline]
pub fn max_object_bytes() -> u64 {
    if target_ptr_size() == 4 {
        i32::MAX as u64
    } else {
        i64::MAX as u64
    }
}

/// Get the target pointer size for the current thread.
pub fn target_ptr_size() -> usize {
    TARGET_PTR_SIZE.with(|c| c.get())
}

/// The builtin typedef (the target's `size_t`) that `__builtin_offsetof`'s
/// expansion casts to: `((__lccc_offsetof_size_t)&((type *)0)->member)`. The
/// reserved name marks the expansion, so the integer-constant-expression
/// rules can accept `offsetof` while rejecting the spelled-out idiom
/// `(size_t)&((T *)0)->m`, as GCC does.
pub const OFFSETOF_SIZE_TYPEDEF: &str = "__lccc_offsetof_size_t";

/// Whether the current target is 32-bit (ILP32).
pub fn target_is_32bit() -> bool {
    target_ptr_size() == 4
}

/// Set the ELF e_machine for the current target. Relocation-type numbers are
/// per-ISA namespaces (R_RISCV_CALL_PLT=19 vs R_X86_64_TLSGD=19), so any
/// classification of a raw relocation number must consult this.
pub fn set_target_elf_machine(machine: u16) {
    TARGET_ELF_MACHINE.with(|c| c.set(machine));
}

/// ELF e_machine of the current target.
/// Human-readable name of the compilation target, for diagnostics that must
/// name the architecture ("`__builtin_ia32_aesenc128` is not available on
/// aarch64").  Mirrors the backend's own `elf_machine_name`, but lives in the
/// common layer so the lowering pipeline can name the target without depending
/// on a backend.
pub fn target_arch_name() -> &'static str {
    match target_elf_machine() {
        3 => "i686",
        40 => "arm",
        62 => "x86-64",
        183 => "aarch64",
        243 => "riscv64",
        _ => "the target architecture",
    }
}

pub fn target_elf_machine() -> u16 {
    TARGET_ELF_MACHINE.with(|c| c.get())
}

/// Set whether the target uses IEEE binary128 for long double (AArch64/RISC-V).
pub fn set_target_long_double_is_f128(is_f128: bool) {
    TARGET_LONG_DOUBLE_IS_F128.with(|c| c.set(is_f128));
}

/// Whether the target uses IEEE binary128 for long double.
/// True for AArch64/RISC-V, false for x86/i686 (x87 80-bit).
pub fn target_long_double_is_f128() -> bool {
    TARGET_LONG_DOUBLE_IS_F128.with(|c| c.get())
}

/// Return the IR type used for pointer-width integers (I64 on LP64, I32 on ILP32).
/// This is the natural "word" type for the target: the size of `long`, `size_t`,
/// and pointer arithmetic results. On 64-bit targets this is I64; on 32-bit (i686)
/// it is I32. Use this instead of hardcoding `IrType::I64` for comparison results,
/// logical operation results, and other values that represent C `int`-class results.
pub fn target_int_ir_type() -> IrType {
    if target_is_32bit() {
        IrType::I32
    } else {
        IrType::I64
    }
}

/// Return the operation type for widened integer arithmetic.
/// On LP64 (64-bit), all integer operations are widened to I64 (the machine word).
/// On ILP32 (i686), operations stay at their C type's natural width:
/// - I32/U32 and smaller → I32 (machine word)
/// - I64/U64 (long long) → I64 (requires register pairs)
/// - I128/U128 → I128
///
/// NOTE: This unconditionally promotes I32/U32 to I64 on 64-bit targets,
/// which means 32-bit operations (multiply, shift, division) use 64-bit
/// instructions. The IntNarrow cast in the codegen handles truncation back
/// to 32-bit semantics. A future improvement would preserve I32/U32 here,
/// but this requires fixing GVN and cfg_simplify to handle I32 types.
pub fn widened_op_type(common_ty: IrType) -> IrType {
    // Float and 128-bit types are never widened; return them unchanged.
    if common_ty.is_float()
        || common_ty == IrType::I128
        || common_ty == IrType::U128
        || common_ty == IrType::Void
    {
        return common_ty;
    }
    match common_ty {
        IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => IrType::I32,
        _ => common_ty,
    }
}

/// Reference-counted string used for struct/union layout keys in CType.
/// Cloning is a cheap reference count increment instead of a heap allocation.
pub type RcStr = Rc<str>;

/// Reference-counted struct layout. Cloning is a cheap reference count
/// increment instead of deep-copying all field names, types, and offsets.
/// This eliminates the most expensive cloning in the lowering phase.
pub type RcLayout = Rc<StructLayout>;

/// Trait for looking up struct/union layout information.
/// TypeContext implements this trait, allowing CType methods in common/
/// to resolve struct/union sizes and alignments without depending on
/// the lowering module directly.
pub trait StructLayoutProvider {
    fn get_struct_layout(&self, key: &str) -> Option<&StructLayout>;
}

/// A HashMap-based provider for struct layouts (used by TypeContext and sema).
impl StructLayoutProvider for FxHashMap<String, RcLayout> {
    fn get_struct_layout(&self, key: &str) -> Option<&StructLayout> {
        self.get(key).map(|rc| rc.as_ref())
    }
}

/// System V AMD64 ABI class of one 8-byte "eightbyte" of a by-value
/// aggregate, as handed from lowering to the backend. Produced only by
/// `crate::common::sysv_classify`, which models the full psABI algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EightbyteClass {
    /// Padding only: occupies no register.
    NoClass,
    /// Passed in the next free SSE register (low 64 bits).
    Sse,
    /// The upper half of the preceding SSE eightbyte's register:
    /// `[Sse, SseUp]` is one 16-byte value in ONE `%xmm` register.
    SseUp,
    /// Passed in the next free general-purpose register.
    Integer,
    /// The aggregate is passed in memory (only ever the sole class:
    /// `[Memory]`). Covers misaligned fields, X87 members and BLKmode
    /// vectors; objects over 16 bytes keep an empty class list instead.
    Memory,
}

/// Classification of a struct for RISC-V LP64D hardware floating-point calling convention.
///
/// The psABI specifies that small structs with specific field patterns should be
/// passed in FP registers rather than GP registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiscvFloatClass {
    /// Struct with exactly one float/double/_Float16/__bf16 member, no other data members.
    /// Passed in a single FP register (fa0-fa7). `size` is 2, 4, or 8 bytes.
    OneFloat { size: u8 },
    /// Struct with exactly two FP members, no other data members.
    /// Passed in two FP registers (fa0-fa7).
    TwoFloats {
        lo_size: u8,
        hi_size: u8,
        hi_offset: usize,
    },
    /// Struct with one FP member + one integer, FP comes first in memory.
    /// FP in FP register, integer in GP register.
    FloatAndInt {
        float_size: u8,
        float_offset: usize,
        int_offset: usize,
        int_size: usize,
    },
    /// Struct with one integer + one FP member, integer comes first in memory.
    /// Integer in GP register, FP in FP register.
    IntAndFloat {
        float_size: u8,
        int_offset: usize,
        int_size: usize,
        float_offset: usize,
    },
}

/// Address space for pointer types (GCC named address space extension).
/// Used for x86 segment-relative memory access (%gs: / %fs: prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AddressSpace {
    #[default]
    Default,
    /// __seg_gs: x86 GS segment (used for per-CPU variables in Linux kernel)
    SegGs,
    /// __seg_fs: x86 FS segment (used for TLS on some platforms)
    SegFs,
}

impl std::fmt::Display for AddressSpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AddressSpace::Default => "generic",
            AddressSpace::SegGs => "__seg_gs",
            AddressSpace::SegFs => "__seg_fs",
        })
    }
}

/// What a [`CType::Pointer`] points at, beyond the pointee's type: the named
/// address space the pointee lives in and the pointee's own qualifiers.
///
/// The qualifiers are `const`/`volatile`/`restrict` of the POINTEE (for an
/// array pointee, of its elements, where C puts them). They are recorded
/// only where the constructing path saw every qualifier written at that
/// level -- a declarator star always does, a `typedef` or `typeof` base may
/// not -- and [`PtrQuals::cvr`] says `None` otherwise. Diagnostics act only
/// on recorded qualifiers, so an unrecorded level can miss a warning but can
/// never invent one.
///
/// Qualifiers change neither representation nor code generation, so
/// equality and hashing look at the address space alone: every `CType`
/// comparison outside the qualifier rules in `frontend::sema` behaves as if
/// the qualifiers were absent.
#[derive(Debug, Clone, Copy, Default)]
pub struct PtrQuals {
    pub space: AddressSpace,
    /// The qualifier bits, plus [`Self::RECORDED`] when they are complete.
    bits: u8,
}

impl PtrQuals {
    pub const CONST: u8 = 1 << 0;
    pub const VOLATILE: u8 = 1 << 1;
    pub const RESTRICT: u8 = 1 << 2;
    const CVR: u8 = Self::CONST | Self::VOLATILE | Self::RESTRICT;
    const RECORDED: u8 = 1 << 7;

    /// A pointer into the generic address space, qualifiers unrecorded.
    pub const GENERIC: Self = Self::of(AddressSpace::Default);

    /// A pointer into `space`, qualifiers unrecorded.
    pub const fn of(space: AddressSpace) -> Self {
        Self { space, bits: 0 }
    }

    /// A pointer into `space` whose pointee carries exactly `cvr`.
    pub const fn recorded(space: AddressSpace, cvr: u8) -> Self {
        Self {
            space,
            bits: (cvr & Self::CVR) | Self::RECORDED,
        }
    }

    /// The pointee's qualifiers, if every one of them was recorded.
    pub const fn cvr(self) -> Option<u8> {
        if self.bits & Self::RECORDED != 0 {
            Some(self.bits & Self::CVR)
        } else {
            None
        }
    }

    /// A proven qualification mismatch. Unknown is not known-unqualified;
    /// it cannot establish incompatibility. Unlike Eq, this is semantic.
    pub const fn has_known_cvr_conflict(self, other: Self) -> bool {
        matches!((self.cvr(), other.cvr()), (Some(a), Some(b)) if a != b)
    }

    /// The same qualifiers on a pointee in `space`.
    #[must_use]
    pub const fn in_space(self, space: AddressSpace) -> Self {
        Self { space, ..self }
    }

    /// The same space with the pointee's qualifiers replaced.
    #[must_use]
    pub const fn with_cvr(self, cvr: Option<u8>) -> Self {
        match cvr {
            Some(cvr) => Self::recorded(self.space, cvr),
            None => Self::of(self.space),
        }
    }
}

impl From<AddressSpace> for PtrQuals {
    fn from(space: AddressSpace) -> Self {
        Self::of(space)
    }
}

impl PartialEq for PtrQuals {
    fn eq(&self, other: &Self) -> bool {
        self.space == other.space
    }
}

impl Eq for PtrQuals {}

impl std::hash::Hash for PtrQuals {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.space.hash(state);
    }
}

// The bits are the parser's (`ast::CVR_*`), so a star's record is stored as is.
const _: () = {
    assert!(PtrQuals::CONST == crate::frontend::parser::ast::CVR_CONST);
    assert!(PtrQuals::VOLATILE == crate::frontend::parser::ast::CVR_VOLATILE);
    assert!(PtrQuals::RESTRICT == crate::frontend::parser::ast::CVR_RESTRICT);
};

/// `PtrQuals` qualifier bits as GCC spells them (`const volatile restrict`).
pub fn cvr_spelling(cvr: u8) -> &'static str {
    const NAMES: [&str; 8] = [
        "",
        "const",
        "volatile",
        "const volatile",
        "restrict",
        "const restrict",
        "volatile restrict",
        "const volatile restrict",
    ];
    NAMES[usize::from(cvr & 0b111)]
}

/// A qualification conflict anywhere in the shared derived shape. Parameter
/// names and top-level parameter CVR are not type identity; pointer records
/// describe the POINTEE and must still be compared inside function signatures.
/// Missing records remain conservative. This must precede qualifier-blind Eq
/// shortcuts in semantic conversion checks.
pub fn recorded_qualifiers_conflict(a: &CType, b: &CType) -> bool {
    match (a, b) {
        (CType::Pointer(a, aq), CType::Pointer(b, bq)) => {
            aq.has_known_cvr_conflict(*bq) || recorded_qualifiers_conflict(a, b)
        }
        (CType::Array(a, _), CType::Array(b, _)) => recorded_qualifiers_conflict(a, b),
        (CType::Function(a), CType::Function(b)) => {
            recorded_qualifiers_conflict(&a.return_type, &b.return_type)
                || (a.has_prototype
                    && b.has_prototype
                    && a.params
                        .iter()
                        .zip(&b.params)
                        .any(|((a, _), (b, _))| recorded_qualifiers_conflict(a, b)))
        }
        _ => false,
    }
}

/// Compatibility of two C types in the sense of `__builtin_types_compatible_p`
/// (C11 6.2.7 "compatible type and composite type").
///
/// This is the ONE definition.  Two earlier copies lived in sema and in the
/// lowering and disagreed: the sema copy compared function types structurally
/// (parameter names are not part of a function type, C11 6.7.6.3p2), the
/// lowering copy used `CType::Function`'s derived `PartialEq`, which compares
/// `FunctionType::params` — and those carry the parameter's NAME because a
/// definition needs it.  A constant initializer is folded by the lowering, so
/// `__builtin_types_compatible_p(void (*)(int), void (*)(int p))` answered 1 in
/// sema's log and 0 in the emitted object: the exact shape the kernel asserts
/// on (`assert_function_type()` in arch/x86/include/asm/irq_stack.h, fed by
/// DEFINE_IDTENTRY_SYSVEC whose parameter is named `regs`).  Whatever the
/// answer is, it must not depend on which half of the compiler asked.
///
/// The rules, each measured against GCC 16.2:
///   * pointers: equal named address space and compatible pointees.  The space
///     is part of the pointer's identity (`int *` and `__seg_gs int *` are
///     incompatible), which the kernel relies on for `__same_type`;
///   * arrays: compatible elements and either equal bounds or one unknown
///     bound (C17 6.2.7p3);
///   * functions: compatible return types; two prototypes must agree in
///     variadic-ness, parameter count and parameter types (names are ignored).
///     A prototype is compatible with an unprototyped type only when it is
///     non-variadic and its parameters are unchanged by default promotions;
///   * enums: compatible with the integer type their value range picks
///     (C11 6.7.2.2p4), so a body-less `enum tag` still matches `int` or
///     `unsigned int` exactly like GCC;
///   * recorded pointee qualification is significant at every pointer level,
///     including function return/parameter types. Unknown records cannot prove
///     a mismatch. Top-level object qualification is not stored in CType;
///   * everything else: structural equality. Unlike ordinary CType Eq, this
///     predicate is for semantic compatibility, not representation identity.
pub fn ctypes_compatible(t1: &CType, t2: &CType) -> bool {
    match (t1, t2) {
        (CType::Pointer(a, sa), CType::Pointer(b, sb)) => {
            sa.space == sb.space && !sa.has_known_cvr_conflict(*sb) && ctypes_compatible(a, b)
        }
        (CType::Array(a, sa), CType::Array(b, sb)) => {
            ctypes_compatible(a, b) && (sa == sb || sa.is_none() || sb.is_none())
        }
        (CType::Function(a), CType::Function(b)) => function_types_compatible(a, b),
        (CType::Enum(e1), CType::Enum(e2)) => {
            e1 == e2
                || (e1.name.is_some()
                    && e1.name == e2.name
                    && (e1.variants.is_empty() || e2.variants.is_empty()))
        }
        (CType::Enum(e), other) | (other, CType::Enum(e)) if other.is_integer() => {
            &e.compatible_ctype() == other
        }
        _ => t1 == t2,
    }
}

/// Type resulting from the default argument promotions (C 6.5.2.2p6).
fn default_argument_promotion(ty: &CType) -> CType {
    match ty {
        CType::Float => CType::Double,
        other => other.integer_promoted(),
    }
}

/// Function-type compatibility (C 6.7.6.3p15). An unprototyped function type
/// is compatible with a prototype only when that prototype is non-variadic
/// and every parameter type is unchanged by the default argument promotions.
/// The `params` stored on an unprototyped K&R definition describe its body,
/// not its visible function type, so they are intentionally ignored here.
fn function_types_compatible(a: &FunctionType, b: &FunctionType) -> bool {
    if !ctypes_compatible(&a.return_type, &b.return_type) {
        return false;
    }

    match (a.has_prototype, b.has_prototype) {
        (true, true) => {
            a.variadic == b.variadic
                && a.params.len() == b.params.len()
                && a.params
                    .iter()
                    .zip(&b.params)
                    .all(|((x, _), (y, _))| ctypes_compatible(x, y))
        }
        (false, false) => !a.variadic && !b.variadic,
        (true, false) => {
            !a.variadic
                && a.params
                    .iter()
                    .all(|(param, _)| ctypes_compatible(param, &default_argument_promotion(param)))
        }
        (false, true) => {
            !b.variadic
                && b.params
                    .iter()
                    .all(|(param, _)| ctypes_compatible(param, &default_argument_promotion(param)))
        }
    }
}

/// Build the composite type of two compatible types for a conditional
/// expression. Unlike compatibility, this must retain information contributed
/// by either operand: in particular, a prototype is the composite of a
/// compatible prototype and an unprototyped function type.
fn composite_compatible_type(a: &CType, b: &CType) -> Option<CType> {
    if !ctypes_compatible(a, b) {
        return None;
    }

    match (a, b) {
        // Compatibility ignores the pointee qualifiers (see `PtrQuals`), so
        // the two records may disagree; the composite keeps only a common one.
        (CType::Pointer(a_inner, a_quals), CType::Pointer(b_inner, b_quals)) => {
            let cvr = a_quals.cvr().filter(|&cvr| b_quals.cvr() == Some(cvr));
            Some(CType::Pointer(
                Box::new(composite_compatible_type(a_inner, b_inner)?),
                a_quals.with_cvr(cvr),
            ))
        }
        (CType::Array(a_elem, a_len), CType::Array(b_elem, b_len)) => Some(CType::Array(
            Box::new(composite_compatible_type(a_elem, b_elem)?),
            a_len.or(*b_len),
        )),
        (CType::Function(a_fn), CType::Function(b_fn)) => Some(CType::Function(Box::new(
            composite_function_type(a_fn, b_fn)?,
        ))),
        _ => Some(a.clone()),
    }
}

fn composite_function_type(a: &FunctionType, b: &FunctionType) -> Option<FunctionType> {
    if !function_types_compatible(a, b) {
        return None;
    }

    let return_type = composite_compatible_type(&a.return_type, &b.return_type)?;
    let (params, variadic, has_prototype) = match (a.has_prototype, b.has_prototype) {
        (true, true) => {
            let params = a
                .params
                .iter()
                .zip(&b.params)
                .map(|((a_ty, a_name), (b_ty, b_name))| {
                    Some((
                        composite_compatible_type(a_ty, b_ty)?,
                        a_name.clone().or_else(|| b_name.clone()),
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            (params, a.variadic, true)
        }
        (true, false) => (a.params.clone(), a.variadic, true),
        (false, true) => (b.params.clone(), b.variadic, true),
        // Parameter names/types on an unprototyped K&R definition describe
        // its body, not the composite function type.
        (false, false) => (Vec::new(), false, false),
    };

    Some(FunctionType {
        return_type,
        params,
        variadic,
        has_prototype,
    })
}

/// Represents C types in the compiler.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CType {
    Void,
    Bool,
    /// Signed 8-bit char: plain `char` where plain char is signed (x86,
    /// i686), explicit `signed char` where it is unsigned (AArch64, RISC-V).
    Char,
    /// Unsigned 8-bit `unsigned char`.
    UChar,
    /// Explicit `signed char` where plain char is signed. Layout-, value- and
    /// promotion-identical to `Char`; differs only in type identity (C11
    /// 6.2.5p15: char, signed char and unsigned char are three distinct types),
    /// which `_Generic`, `__builtin_types_compatible_p` and pointer
    /// compatibility observe. Every pattern naming `Char` also names `SChar`.
    SChar,
    /// Plain `char` where plain char is unsigned: the `UChar` counterpart of
    /// `SChar`. Every pattern naming `UChar` also names `PlainUChar`.
    PlainUChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
    Int128,
    UInt128,
    Float,
    Double,
    LongDouble,
    /// ISO C23 `_Float16` / `__fp16`: IEEE binary16, 2 bytes, 2-byte aligned.
    /// Carried in IR as `U16` (with FP-register ABI flags on calls/params/returns).
    Float16,
    /// GCC/Clang `__bf16`: brain float16, 2 bytes, 2-byte aligned.
    /// Carried in IR as `U16` (with FP-register ABI flags on calls/params/returns).
    BFloat16,
    /// ISO C _Float128 / __float128: IEEE binary128, 16 bytes.
    /// Distinct from long double (80-bit x87 on x86-64).
    Float128,
    /// ISO C23 / TS 18661-2 decimal floating point (_Decimal32), IEEE 754-2008
    /// BID encoding, 7 decimal digits, 4 bytes. Values are bit containers;
    /// every operation is lowered to libbid (__bid_*) helper calls at the
    /// CType level, mirroring the Float128 strategy.
    Decimal32,
    /// _Decimal64: 16 decimal digits, 8 bytes, BID encoding.
    Decimal64,
    /// _Decimal128: 34 decimal digits, 16 bytes, BID encoding. Carried in IR
    /// as U128 (bit-exact 16-byte moves) with the Float128 SSE ABI flags.
    Decimal128,
    /// C99 _Complex float: two f32 values (real, imag)
    ComplexFloat,
    /// C99 _Complex double: two f64 values (real, imag)
    ComplexDouble,
    /// C99 _Complex long double: two f128 values (real, imag) - uses F128 storage per component
    ComplexLongDouble,
    /// A pointer to the boxed type; see [`PtrQuals`].
    Pointer(Box<CType>, PtrQuals),
    /// C23 `nullptr_t` (6.2.5p25), the type of `nullptr`: an unqualified
    /// scalar with the size, alignment and representation of `void *` whose
    /// only value is the null pointer. It converts to any pointer type and
    /// to `bool`, compares with pointers, and is never arithmetic.
    NullPtr,
    Array(Box<CType>, Option<usize>),
    Function(Box<FunctionType>),
    /// Struct type, identified by key (e.g., "struct.Foo" or "__anon_struct_7").
    /// The actual layout is stored in TypeContext's struct_layouts map.
    /// Uses Rc<str> for cheap cloning (reference count bump vs heap allocation).
    Struct(RcStr),
    /// Union type, identified by key (e.g., "union.Bar" or "__anon_struct_7").
    /// The actual layout is stored in TypeContext's struct_layouts map.
    /// Uses Rc<str> for cheap cloning (reference count bump vs heap allocation).
    Union(RcStr),
    Enum(EnumType),
    /// GCC vector extension type: __attribute__((vector_size(N))).
    /// Stores (element_type, total_size_in_bytes). The byte-size contract is
    /// intentionally distinct from Clang's element-count-based ext_vector_type.
    /// E.g., `typedef int v4si __attribute__((vector_size(16)))` -> Vector(Int, 16).
    Vector(Box<CType>, usize),
    /// Clang vector extension type: __attribute__((ext_vector_type(N))).
    /// The layout stores logical lanes separately from padded object storage;
    /// Bool lanes are packed as bits. Do not infer lane count from storage size.
    ExtVector(Box<CType>, ExtVectorLayout),
}

/// Clang 23.1.3's i386 carrier for a by-value `ext_vector_type` value,
/// derived with the executable probes in
/// `tests/regression/ext_vector_abi/i386_probe` (LLVM's i386 calling
/// convention applied to Clang's IR vector type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I386ExtVectorCarrier {
    /// Argument: each lane is its own scalar argument in its own stack slot.
    Lanes,
    /// Argument: the 8-byte memory image in one stack slot (Clang coerces a
    /// 64-bit integer-lane vector to `i64`).
    Image64,
    /// 16-byte pieces in XMM registers, lanes widened in place (`_Bool`
    /// lanes become 16/N-byte integers whose bit 0 is the lane).
    /// Arguments: %xmm0-%xmm2 then 16-byte-aligned stack slots; returns: %xmm0.
    Xmm { pieces: usize },
    /// Return: the lone lane as a scalar (%eax, %eax:%edx or %st(0)).
    Scalar,
    /// Return: lane i in the i-th of %eax, %edx, %ecx (at most three integer
    /// lanes of at most 4 bytes).
    GpLanes,
    /// Return: through a hidden pointer to the memory image.
    Sret,
    /// Clang's convention has no lccc carrier (several XMM or x87 return
    /// registers, or one-lane-per-slot `_Bool` vectors over 64 lanes).
    Unsupported,
}

/// Target-specific object layout for a Clang `ext_vector_type`.
///
/// `vector_size` retains its legacy byte-width semantics in `CType::Vector`.
/// This descriptor is only used for `ext_vector_type`, whose object may be
/// padded beyond its logical lanes and whose `_Bool` lanes are bit-packed.

/// Byte stride of a `long double` lane inside an ext vector: LLVM packs
/// `<N x x86_fp80>` at the element's 80-bit size (see `ExtVectorLayout`).
pub const X87_LANE_STRIDE: usize = 10;

/// Why a Clang ext-vector element selector is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtVectorSelectorError {
    /// `illegal vector component name '...'`, quoting the selector bytes in
    /// this range (char boundaries): the offending character, or the whole
    /// selector on a `_Bool` vector. Clang places the caret `range.start`
    /// bytes after the byte that follows the `.`/`->` operator's first byte.
    IllegalName(std::ops::Range<usize>),
    /// `vector component access exceeds type ...`, at the operator.
    ExceedsType,
}

impl ExtVectorSelectorError {
    /// Clang 23.1.3's message for this error in `<vector>.<selector>`.
    /// Clang spells the vector by its typedef name where lccc has none.
    pub fn message(&self, selector: &str, vector: &CType) -> String {
        match self {
            ExtVectorSelectorError::IllegalName(range) => {
                let name = selector.get(range.clone()).unwrap_or(selector);
                format!("illegal vector component name '{name}'")
            }
            ExtVectorSelectorError::ExceedsType => {
                let (elem, lanes) = vector.vector_info().map_or_else(
                    || (String::new(), 0),
                    |(elem, lanes)| (elem.to_string(), lanes),
                );
                let plural = if lanes == 1 { "" } else { "s" };
                format!(
                    "vector component access exceeds type '{vector}' \
                     (vector of {lanes} '{elem}' value{plural})"
                )
            }
        }
    }
}

/// The lanes an ext-vector element selector (`v.wzyx`, `v.s3`, `v.hi`) names
/// on a vector of `lanes` logical lanes, in result order.
///
/// Rules measured against Clang 23.1.3 (`Sema::CheckExtVectorComponent`):
/// the first character picks one accessor set, `xyzw` or `rgba` (lanes
/// 0-3), or `s`/`S` followed by one or more hex digits; the first character
/// outside that set is the illegal one; every named lane must be a logical
/// lane. The whole names `hi`, `lo`, `even` and `odd` select
/// `(lanes + 1) / 2` lanes: `lo` from 0, `hi` from `(lanes + 1) / 2`,
/// `even`/`odd` every other lane. Those may name the padding lane of an
/// odd-length vector (`v3.hi` is lanes 2 and 3) and, on a one-lane vector,
/// lane 1, outside the object: Clang reads such a lane as poison, and the
/// lane helpers read it as zero and drop stores to it without touching
/// memory. Selectors may repeat lanes and have any length.
pub fn ext_vector_selector_lanes(
    name: &str,
    lanes: usize,
) -> Result<Vec<usize>, ExtVectorSelectorError> {
    let half = lanes.div_ceil(2);
    // (first lane, stride) of a halving selector.
    let halving = match name {
        "lo" => Some((0, 1)),
        "hi" => Some((half, 1)),
        "even" => Some((0, 2)),
        "odd" => Some((1, 2)),
        _ => None,
    };
    if let Some((first, stride)) = halving {
        return Ok((0..half).map(|i| first + stride * i).collect());
    }
    let illegal = |at: usize, c: char| ExtVectorSelectorError::IllegalName(at..at + c.len_utf8());
    let Some(first) = name.chars().next() else {
        return Err(ExtVectorSelectorError::IllegalName(0..0));
    };
    let numeric = matches!(first, 's' | 'S');
    let lane_of: fn(char) -> Option<usize> = if numeric {
        |c| c.to_digit(16).map(|d| d as usize)
    } else if "xyzw".contains(first) {
        |c| "xyzw".find(c)
    } else if "rgba".contains(first) {
        |c| "rgba".find(c)
    } else {
        return Err(illegal(0, first));
    };
    // Byte offset of the first component: `s`/`S` is a prefix, not a lane.
    let start = if numeric { first.len_utf8() } else { 0 };
    if start == name.len() {
        return Err(illegal(0, first));
    }
    let mut selected = Vec::with_capacity(name.len() - start);
    for (at, c) in name[start..].char_indices() {
        selected.push(lane_of(c).ok_or_else(|| illegal(start + at, c))?);
    }
    if selected.iter().any(|&lane| lane >= lanes) {
        return Err(ExtVectorSelectorError::ExceedsType);
    }
    Ok(selected)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExtVectorLayout {
    pub lanes: usize,
    pub storage_size: usize,
    pub alignment: usize,
    /// Byte stride of a non-bitpacked lane. `long double` lanes are the
    /// LLVM vector layout of `<N x x86_fp80>`: elements are bit-packed at
    /// their 80-bit *size*, not their 16/12-byte alloc size, so the stride
    /// is 10 on both x86-64 and i686 (`X87_LANE_STRIDE`). The object size
    /// still follows Clang's AST (`N * sizeof(long double)` rounded up to a
    /// power of two: 16/32/64), which is why a `<3 x x86_fp80>` global is a
    /// 32-byte ELF object inside a 64-byte C object.
    pub element_storage_size: usize,
    /// Clang stores `_Bool` ext-vector lanes as consecutive bits, least
    /// significant lane first within each byte.
    pub bitpacked_bool: bool,
}

impl ExtVectorLayout {
    /// Build the ext-vector layout for the target lccc currently emits code
    /// for (x86-32 and x86-64). The power-of-two padded size and bit-packed
    /// `_Bool` lanes are measured against clang-19 on Intel; the 10-byte
    /// x87 lane stride is LLVM's `<N x x86_fp80>` layout (the earlier
    /// 16-byte stride was inferred from `sizeof` alone and made every
    /// multi-lane `long double` vector disagree with Clang in memory and
    /// across calls -- ABI-EXTVEC-LD-1). Returns None for invalid lane types, zero
    /// lanes, unsupported backends, or size overflow; callers are expected to
    /// treat None as "attribute not recognized on this target" and fall back
    /// to the scalar element (matching GCC's unknown-attribute semantics).
    ///
    /// When ARM or RISC-V support for `ext_vector_type` lands, this constructor
    /// must branch on `target_elf_machine()` (or grow a per-backend hook)
    /// because both targets use different padding/alignment rules; the current
    /// x86 numbers will silently miscompile those ABIs.
    pub fn new(element: &CType, lanes: usize) -> Option<Self> {
        // The layout arithmetic below is only validated for x86-64 (EM_386=3,
        // EM_X86_64=62). Other backends must provide their own rules.
        let machine = crate::common::types::target_elf_machine();
        if machine != 3 && machine != 62 {
            return None;
        }
        if lanes == 0 {
            return None;
        }
        let bitpacked_bool = matches!(element, CType::Bool);
        let element_storage_size = if bitpacked_bool {
            0
        } else {
            match element {
                CType::Char
                | CType::SChar
                | CType::UChar
                | CType::PlainUChar
                | CType::Short
                | CType::UShort
                | CType::Int
                | CType::UInt
                | CType::Long
                | CType::ULong
                | CType::LongLong
                | CType::ULongLong
                | CType::Int128
                | CType::UInt128
                | CType::Float
                | CType::Double
                | CType::LongDouble => {
                    if matches!(element, CType::LongDouble) {
                        X87_LANE_STRIDE
                    } else {
                        element.size()
                    }
                }
                _ => return None,
            }
        };
        // The C object size is Clang's AST size: lanes times the scalar
        // `sizeof` (16 on LP64, 12 on i686 for `long double`), rounded up
        // to a power of two. For x87 lanes that is NOT the lane stride.
        let payload_size = if bitpacked_bool {
            lanes.checked_add(7)?.checked_div(8)?
        } else {
            lanes.checked_mul(element.size())?
        };
        let storage_size = payload_size.checked_next_power_of_two()?;
        if storage_size == 0 {
            return None;
        }
        Some(Self {
            lanes,
            storage_size,
            alignment: storage_size,
            element_storage_size,
            bitpacked_bool,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionType {
    pub return_type: CType,
    pub params: Vec<(CType, Option<String>)>,
    pub variadic: bool,
    /// Whether this type has a prototype. An empty parameter vector alone is
    /// insufficient: `f()` is unprototyped before C23 but means zero parameters
    /// in C23, while `f(void)` is a zero-parameter prototype in every dialect.
    /// For K&R definitions, body parameters live in `FunctionInfo::params`; the
    /// callable `FunctionType` remains unprototyped unless an earlier prototype
    /// is visible.
    pub has_prototype: bool,
}

/// StructField describes a single field in a struct or union declaration.
/// Used during construction of StructLayout.

/// The alignment a member or object gets from all of its explicit sources
/// (`_Alignas(N)`, `_Alignas(type)`, `aligned(N)`, an aligned typedef): the
/// strictest one (C11 6.7.5p6), or `None` when there is none. Every
/// `StructFieldDecl` -> `StructField` conversion merges through this, so the
/// sema and lowering layouts of one record cannot differ.
pub fn strictest_alignment(sources: impl IntoIterator<Item = Option<usize>>) -> Option<usize> {
    sources.into_iter().flatten().max()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructField {
    pub name: String,
    pub ty: CType,
    pub bit_width: Option<u32>,
    /// Per-field alignment request from `_Alignas(N)` or
    /// `__attribute__((aligned(N)))` on the member itself. Can only RAISE the
    /// member's alignment above its type's (GCC: a member `aligned` below the
    /// type alignment is ignored unless `packed`).
    pub alignment: Option<usize>,
    /// The field type's own `TYPE_ALIGN` when it comes from a typedef that
    /// carries `__attribute__((aligned(N)))`: REPLACES the natural alignment
    /// (GCC: "when used on a typedef, the aligned attribute can both increase
    /// and decrease alignment" — `typedef double D4 aligned(4)` makes
    /// `struct { char c; D4 d; }` 12 bytes with `d` at 4; GCC 16.2 -m64/-m32).
    /// A member-level `alignment` still raises above this.
    pub type_align: Option<usize>,
    /// Per-field __attribute__((packed)) - forces this field's alignment to 1.
    pub is_packed: bool,
    /// Volatile base type of the field (element or pointee access). See `StructFieldDecl`.
    pub base_volatile: bool,
    /// Const on the field's base type (element or innermost pointee).
    pub base_const: bool,
    /// Volatile field object itself (`int *volatile p`). See `StructFieldDecl`.
    pub object_volatile: bool,
    /// Const on the field object itself, independent of its pointee.
    pub object_const: bool,
    /// Optional sibling count member from `__attribute__((counted_by(member)))`.
    pub counted_by: Option<String>,
    /// CVR bits of each pointer level of the field's type, in CType creation
    /// order (index `L-1` is pointer level `L`). `CType` erases qualifiers, so
    /// a middle level's `const` (`int *const *p` as a member) lives here.
    pub pointer_quals: Vec<u8>,
    /// The member object's complete own CVR bits (an array member: its
    /// elements'), when exactly known: sema records them for a star-free
    /// declarator whose specifier qualifiers are completely known (the same
    /// rule as a variable's object record). `None` is "unknown", never "none":
    /// `&s.m` and the decay of an array member then stay unrecorded.
    pub own_cvr: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumType {
    pub name: Option<String>,
    pub variants: Vec<(String, i64)>,
    /// When true (__attribute__((packed))), the enum uses the smallest
    /// integer type that can represent all variant values.
    pub is_packed: bool,
    /// C23 fixed underlying type (`enum E : T`, 6.7.2.2p5): the enum is
    /// compatible with it, has its size and alignment, and its enumerators
    /// have the enumerated type. Overrides `is_packed` and the value rules.
    pub fixed: Option<Box<CType>>,
}

impl EnumType {
    /// The fixed underlying type `: T` names, if it is a valid one (C23
    /// 6.7.2.2p5: an integer type other than an enumerated type; its
    /// qualifiers are dropped, and `CType` carries none).
    pub fn fixed_underlying(ty: CType) -> Option<CType> {
        (ty.is_integer() && !matches!(ty, CType::Enum(_))).then_some(ty)
    }

    /// The integer type this enumeration is *compatible* with.
    ///
    /// C11 6.7.2.2p4: "Each enumerated type shall be compatible with char, a
    /// signed integer type, or an unsigned integer type. The choice of type is
    /// implementation-defined". GCC's choice — which the Linux kernel depends
    /// on — is `unsigned int` when no enumerator is negative and all values fit
    /// 32 bits, `int` when some are negative, and a 64-bit type otherwise.
    ///
    /// This is not pedantry: pointers to compatible types are compatible, so
    /// `unsigned int *` and `enum pg_level *` must interchange. The kernel does
    /// exactly that — arch/x86/mm/pat/set_memory.c passes `enum pg_level level`
    /// to `lookup_address(addr, unsigned int *level)` (declared in
    /// arch/x86/include/asm/pgtable_types.h) — and GCC accepts it even under
    /// -Werror=incompatible-pointer-types, while lccc rejected it and stopped
    /// the build at arch/x86/mm/pat/set_memory.o.
    ///
    /// Signedness follows the presence of a negative enumerator, and the width
    /// follows `packed_size` so a packed enum's compatible type is the same
    /// smallest type it occupies.
    pub fn compatible_ctype(&self) -> CType {
        if let Some(fixed) = &self.fixed {
            return (**fixed).clone();
        }
        let any_negative = self.variants.iter().any(|(_, v)| *v < 0);
        match self.packed_size() {
            1 => {
                if any_negative {
                    // GCC c_common_type_for_size(8, signed): `signed char`
                    CType::signed_char()
                } else {
                    CType::UChar
                }
            }
            2 => {
                if any_negative {
                    CType::Short
                } else {
                    CType::UShort
                }
            }
            // GCC and Clang pick `long` on LP64 targets and `long long` on
            // ILP32 (measured: `_Generic` on `enum { E = 1LL << 32 }`).
            8 => match (any_negative, target_ptr_size() == 8) {
                (true, true) => CType::Long,
                (false, true) => CType::ULong,
                (true, false) => CType::LongLong,
                (false, false) => CType::ULongLong,
            },
            _ => {
                if any_negative {
                    CType::Int
                } else {
                    CType::UInt
                }
            }
        }
    }

    /// Type of every enumerator once the enum is complete: `int` when all
    /// enumerators fit in `int`, otherwise the enum's compatible type for all
    /// of them (`enum { A = 0, B = 1LL << 32 }` makes `A` an `unsigned long`
    /// on LP64). GCC 16.2 and Clang 23.1.3 agree in every `-std` mode
    /// (measured with `_Generic` and `sizeof` under gnu17, gnu23 and c11).
    pub fn constant_ctype(&self) -> CType {
        if self.fixed.is_some() {
            return CType::Enum(self.clone());
        }
        if self.variants.iter().all(|(_, v)| i32::try_from(*v).is_ok()) {
            CType::Int
        } else {
            self.compatible_ctype()
        }
    }

    /// Returns the size (and alignment) in bytes for this enum.
    /// Non-packed enums are always 4 bytes (int). Packed enums use the
    /// smallest integer type that can represent all variant values.
    pub fn packed_size(&self) -> usize {
        if let Some(fixed) = &self.fixed {
            return fixed.size();
        }
        if !self.is_packed {
            // Non-packed enum: 4 bytes when ONE 32-bit type holds every value
            // (`unsigned int` without negatives, else `int`), otherwise 8 (GCC
            // extension). `{ -1, 0x80000000 }` fits neither 32-bit type and is
            // `long` (GCC 16.2 and Clang 23.1.3, measured).
            let any_negative = self.variants.iter().any(|(_, v)| *v < 0);
            let fits_32 = self.variants.iter().all(|(_, v)| {
                if any_negative {
                    i32::try_from(*v).is_ok()
                } else {
                    u32::try_from(*v).is_ok()
                }
            });
            return if fits_32 { 4 } else { 8 };
        }
        // Find the range of variant values to determine the minimum size
        if self.variants.is_empty() {
            // Packed enum with no known variants -- default to 1 byte
            return 1;
        }
        let min_val = self.variants.iter().map(|(_, v)| *v).min().unwrap();
        let max_val = self.variants.iter().map(|(_, v)| *v).max().unwrap();
        if min_val >= 0 {
            // Unsigned range
            if max_val <= 0xFF {
                1
            } else if max_val <= 0xFFFF {
                2
            } else if max_val <= 0xFFFF_FFFF {
                4
            } else {
                8
            }
        } else {
            // Signed range
            if min_val >= -128 && max_val <= 127 {
                1
            } else if min_val >= -32768 && max_val <= 32767 {
                2
            } else if min_val >= i32::MIN as i64 && max_val <= i32::MAX as i64 {
                4
            } else {
                8
            }
        }
    }
}

/// Computed layout for a struct or union, with field offsets and total size.
#[derive(Debug, Clone)]
pub struct StructLayout {
    /// True for a record definition, including a GNU empty record; false for a forward placeholder.
    pub is_complete: bool,
    /// Each field's (name, byte offset, CType).
    pub fields: Vec<StructFieldLayout>,
    /// Total size of the struct in bytes (including trailing padding).
    pub size: usize,
    /// Required alignment of the struct.
    pub align: usize,
    /// Whether this is a union (all fields at offset 0).
    pub is_union: bool,
    /// Whether this union has `__attribute__((transparent_union))`.
    /// A transparent union parameter is passed using the ABI of its first member.
    pub is_transparent_union: bool,
    /// True when the record has `__attribute__((scalar_storage_order("big-endian")))`
    /// on a little-endian target (or the mirrored little-endian form on a
    /// big-endian one) — i.e. the storage order is REVERSED relative to the
    /// target's native byte order.
    pub reverse_sso: bool,
    /// GCC `TYPE_USER_ALIGN`: the record's alignment was raised by an
    /// explicit request — a struct-level `aligned(N)` (any N), a field with
    /// `_Alignas`/`aligned` at or above its type's natural alignment, or a
    /// field whose type is itself user-aligned (nested record, array of
    /// one). C11 `_Alignof` does not apply the `BIGGEST_ALIGNMENT` cap to
    /// user-aligned types (`min_align_of_type`), so `_Alignof(struct {
    /// _Alignas(32) int x; })` is 32 while `_Alignof(struct { v32 v; })`
    /// is 16 without AVX. A field whose explicit alignment is BELOW its
    /// type's does not count: GCC's `do_type_align` overwrites that
    /// field's `DECL_USER_ALIGN` with the type's (measured: `struct { v32 v
    /// __attribute__((aligned(8))); }` → `_Alignof` 16).
    pub user_align: bool,
}

/// Builder for struct layout computation with bitfield state tracking.
struct StructLayoutBuilder {
    offset: usize,
    max_align: usize,
    field_layouts: Vec<StructFieldLayout>,
    bf_unit_offset: usize,
    bf_bit_pos: u32,
    bf_unit_size: usize,
    in_bitfield: bool,
    is_packed_1: bool,
    reverse_sso: bool,
    /// See `StructLayout::user_align`.
    user_align: bool,
}

impl StructLayoutBuilder {
    fn new(field_count: usize, max_field_align: Option<usize>, reverse_sso: bool) -> Self {
        Self {
            offset: 0,
            max_align: 1,
            field_layouts: Vec::with_capacity(field_count),
            bf_unit_offset: 0,
            bf_bit_pos: 0,
            bf_unit_size: 0,
            in_bitfield: false,
            is_packed_1: max_field_align == Some(1),
            reverse_sso,
            user_align: false,
        }
    }

    /// Compute the effective alignment and size for a field.
    fn compute_field_alignment(
        &mut self,
        field: &StructField,
        max_field_align: Option<usize>,
        ctx: &dyn StructLayoutProvider,
    ) -> (usize, usize) {
        let natural_align = field.type_align.unwrap_or_else(|| field.ty.align_ctx(ctx));
        let field_align = if field.is_packed {
            // Per-field __attribute__((packed)) forces alignment to 1 unless an
            // explicit aligned(N) / _Alignas(N) attribute also sets floor alignment.
            field.alignment.unwrap_or(1)
        } else if let Some(explicit) = field.alignment {
            // Explicit _Alignas/aligned raises alignment, overrides struct-level packing
            natural_align.max(explicit)
        } else if let Some(max_a) = max_field_align {
            // Struct-level packed or #pragma pack caps alignment
            natural_align.min(max_a)
        } else {
            natural_align
        };
        let field_size = field.ty.size_ctx(ctx);
        self.max_align = self.max_align.max(field_align);
        self.user_align |= field_contributes_user_align(field, natural_align, ctx);
        (field_align, field_size)
    }

    /// Handle zero-width bitfield: force alignment to next storage unit boundary.
    fn layout_zero_width_bitfield(&mut self, field_align: usize) {
        if self.in_bitfield {
            if self.is_packed_1 {
                let total_bits = (self.bf_unit_offset * 8) as u32 + self.bf_bit_pos;
                self.offset = align_up((total_bits as usize).div_ceil(8), 1);
            } else {
                self.offset = self.bf_unit_offset + self.bf_unit_size;
            }
        }
        self.offset = align_up(self.offset, field_align);
        self.in_bitfield = false;
        self.bf_bit_pos = 0;
    }

    /// Layout a bitfield with pack(1) — contiguous bit stream, may span storage units.
    fn layout_packed_bitfield(&mut self, field: &StructField, bw: u32, field_size: usize) {
        let unit_bits = (field_size * 8) as u32;
        if !self.in_bitfield {
            self.bf_unit_offset = self.offset;
            self.bf_bit_pos = 0;
            self.bf_unit_size = field_size;
            self.in_bitfield = true;
        }
        let total_bit_offset = (self.bf_unit_offset * 8) as u32 + self.bf_bit_pos;
        let storage_offset = (total_bit_offset / 8) as usize;
        let bit_offset_in_storage = total_bit_offset % 8;

        // Widen storage type if bitfield spans beyond declared type.
        let needed_bits = bit_offset_in_storage + bw;
        let storage_ty = if needed_bits > unit_bits {
            let needed_bytes = needed_bits.div_ceil(8) as usize;
            let is_signed = field.ty.is_signed();
            StructLayout::smallest_int_ctype_for_bytes(needed_bytes, is_signed)
        } else {
            field.ty.clone()
        };

        // Reverse SSO + packed: the bit stream fills each byte MSB-first.
        // Layout offsets stay logical; extraction applies per-byte bit-reverse.
        let sso = if self.reverse_sso {
            SsoMode::ByteBitReverse
        } else {
            SsoMode::None
        };

        self.field_layouts.push(StructFieldLayout {
            name: field.name.clone(),
            base_volatile: field.base_volatile,
            base_const: field.base_const,
            object_volatile: field.object_volatile,
            object_const: field.object_const,
            counted_by: field.counted_by.clone(),
            pointer_quals: field.pointer_quals.clone(),
            own_cvr: field.own_cvr,
            offset: storage_offset,
            ty: storage_ty,
            bit_offset: Some(bit_offset_in_storage),
            bit_width: Some(bw),
            sso,
            sso_storage_ty: None,
        });
        self.bf_bit_pos += bw;
    }

    /// Layout a standard (non-packed) bitfield — SysV ABI compatible.
    fn layout_standard_bitfield(
        &mut self,
        field: &StructField,
        bw: u32,
        field_size: usize,
        field_align: usize,
    ) {
        let unit_bits = (field_size * 8) as u32;

        // Compute absolute bit position of the cursor
        let abs_bit_pos: u64 = if self.in_bitfield {
            (self.bf_unit_offset as u64) * 8 + self.bf_bit_pos as u64
        } else {
            (self.offset as u64) * 8
        };

        // Check if the bitfield would straddle a boundary of its own type
        let type_bits = unit_bits as u64;
        let straddles = if type_bits > 0 && bw > 0 {
            abs_bit_pos / type_bits != (abs_bit_pos + bw as u64 - 1) / type_bits
        } else {
            true
        };

        // Determine placement bit position
        let placed_abs_bit = if !self.in_bitfield {
            if straddles {
                (align_up(self.offset, field_align) as u64) * 8
            } else {
                abs_bit_pos
            }
        } else if straddles {
            self.offset = self.bf_unit_offset + self.bf_unit_size;
            (align_up(self.offset, field_align) as u64) * 8
        } else {
            abs_bit_pos
        };

        // Compute storage unit offset.
        // New placement uses ABI alignment; continuation uses sizeof.
        let storage_mask = if !self.in_bitfield || straddles {
            if field_align > 0 { field_align } else { 1 }
        } else if field_size > 0 {
            field_size
        } else {
            1
        };
        let field_storage_offset = ((placed_abs_bit / 8) as usize) & !(storage_mask - 1);
        let mut field_bit_in_storage = (placed_abs_bit - (field_storage_offset as u64) * 8) as u32;

        // Reverse scalar_storage_order: bitfields are numbered from the MOST
        // significant bit of the storage unit (GCC stor-layout semantics), and
        // the unit itself is stored byte-reversed in memory. Mirror the bit
        // position within the unit; for multi-byte units the access loads the
        // whole unit (sso_storage_ty) and byteswaps it before bit math.
        let unit_bits = (storage_mask * 8) as u32;
        let (sso, sso_storage_ty, field_bit_in_storage, _storage_ctype) = if self.reverse_sso {
            field_bit_in_storage = unit_bits - field_bit_in_storage - bw;
            if storage_mask >= 2 {
                let unit_ctype =
                    StructLayout::smallest_int_ctype_for_bytes(storage_mask, field.ty.is_signed());
                (
                    SsoMode::ByteSwapUnit(storage_mask as u32),
                    Some(unit_ctype.clone()),
                    field_bit_in_storage,
                    unit_ctype,
                )
            } else {
                // 1-byte unit: no byte to swap; the reversed bit offset alone
                // gives the MSB-first numbering inside the byte.
                (SsoMode::None, None, field_bit_in_storage, field.ty.clone())
            }
        } else {
            (SsoMode::None, None, field_bit_in_storage, field.ty.clone())
        };

        self.field_layouts.push(StructFieldLayout {
            name: field.name.clone(),
            base_volatile: field.base_volatile,
            base_const: field.base_const,
            object_volatile: field.object_volatile,
            object_const: field.object_const,
            counted_by: field.counted_by.clone(),
            pointer_quals: field.pointer_quals.clone(),
            own_cvr: field.own_cvr,
            offset: field_storage_offset,
            ty: field.ty.clone(),
            bit_offset: Some(field_bit_in_storage),
            bit_width: Some(bw),
            sso,
            sso_storage_ty,
        });

        // Update bitfield tracking state
        let field_end_byte = field_storage_offset + field_size;
        if !self.in_bitfield || straddles {
            self.bf_unit_offset = field_storage_offset;
            self.bf_unit_size = field_size;
        } else if field_end_byte > self.bf_unit_offset + self.bf_unit_size {
            self.bf_unit_size = field_end_byte - self.bf_unit_offset;
        }
        let new_bf_bit_pos = placed_abs_bit + bw as u64 - (self.bf_unit_offset as u64) * 8;
        debug_assert!(
            new_bf_bit_pos <= u32::MAX as u64,
            "bitfield bit position overflow"
        );
        self.bf_bit_pos = new_bf_bit_pos as u32;
        self.in_bitfield = true;
    }

    /// Layout a regular (non-bitfield) field.
    fn layout_regular_field(&mut self, field: &StructField, field_size: usize, field_align: usize) {
        if self.in_bitfield {
            let total_bits = (self.bf_unit_offset * 8) as u32 + self.bf_bit_pos;
            self.offset = (total_bits as usize).div_ceil(8);
            self.in_bitfield = false;
        }

        self.offset = align_up(self.offset, field_align);

        // Reverse SSO: scalar members are stored byte-reversed. Mark every
        // multi-byte scalar for a byteswap at access (1-byte members are
        // order-neutral; aggregates carry their own per-record order).
        let sso = if self.reverse_sso && field_size >= 2 {
            SsoMode::ByteSwapUnit(field_size as u32)
        } else {
            SsoMode::None
        };

        self.field_layouts.push(StructFieldLayout {
            name: field.name.clone(),
            base_volatile: field.base_volatile,
            base_const: field.base_const,
            object_volatile: field.object_volatile,
            object_const: field.object_const,
            counted_by: field.counted_by.clone(),
            pointer_quals: field.pointer_quals.clone(),
            own_cvr: field.own_cvr,
            offset: self.offset,
            ty: field.ty.clone(),
            bit_offset: None,
            bit_width: None,
            sso,
            sso_storage_ty: None,
        });

        let is_flexible_array = matches!(&field.ty, CType::Array(_, None));
        if !is_flexible_array {
            self.offset += field_size;
        }
    }

    /// Finalize the layout: account for trailing bitfield and padding.
    fn finalize(mut self) -> StructLayout {
        if self.in_bitfield {
            let total_bits = (self.bf_unit_offset * 8) as u32 + self.bf_bit_pos;
            self.offset = (total_bits as usize).div_ceil(8);
        }

        let size = align_up(self.offset, self.max_align);
        StructLayout {
            is_complete: true,
            fields: self.field_layouts,
            size,
            align: self.max_align,
            is_union: false,
            is_transparent_union: false,
            reverse_sso: self.reverse_sso,
            user_align: self.user_align,
        }
    }
}

/// GCC `DECL_USER_ALIGN` of a record field as it feeds `TYPE_USER_ALIGN`
/// of the enclosing record (`update_alignment_for_field` after
/// `layout_decl`/`do_type_align`). See `StructLayout::user_align`.
fn field_contributes_user_align(
    field: &StructField,
    natural_align: usize,
    ctx: &dyn StructLayoutProvider,
) -> bool {
    if field.is_packed {
        return field.alignment.is_some();
    }
    // A typedef-level `aligned` sets TYPE_USER_ALIGN on the typedef'd type.
    if field.type_align.is_some() {
        return true;
    }
    match field.alignment {
        Some(explicit) if explicit >= natural_align => true,
        _ => field.ty.has_user_align(ctx),
    }
}

impl StructLayout {
    /// Create an empty StructLayout (zero-size, no fields).
    /// Used as a fallback when a struct/union layout is not found.
    pub fn empty() -> Self {
        StructLayout {
            is_complete: false,
            fields: Vec::new(),
            size: 0,
            align: 1,
            is_union: false,
            is_transparent_union: false,
            reverse_sso: false,
            user_align: false,
        }
    }

    /// Create an empty union StructLayout (zero-size, no fields, is_union=true).
    pub fn empty_union() -> Self {
        StructLayout {
            is_complete: false,
            fields: Vec::new(),
            size: 0,
            align: 1,
            is_union: true,
            is_transparent_union: false,
            reverse_sso: false,
            user_align: false,
        }
    }

    /// Apply a struct-level `__attribute__((aligned(N)))` / `_Alignas`
    /// request: raises the record alignment (and re-rounds its size) when
    /// `N` exceeds the natural alignment, and marks the record user-aligned
    /// in either case (GCC sets `TYPE_USER_ALIGN` for any explicit request,
    /// so `_Alignof(struct __attribute__((aligned(8))) { v32 v; })` is 32,
    /// uncapped, while the attribute-less record reports 16 without AVX).
    pub fn apply_struct_aligned(&mut self, struct_aligned: Option<usize>) {
        let Some(a) = struct_aligned else {
            return;
        };
        self.user_align = true;
        if a > self.align {
            self.align = a;
            self.size = align_up(self.size, a);
        }
    }

    /// Create an Rc-wrapped empty StructLayout.
    pub fn empty_rc() -> RcLayout {
        Rc::new(Self::empty())
    }

    /// Create an Rc-wrapped empty union StructLayout.
    pub fn empty_union_rc() -> RcLayout {
        Rc::new(Self::empty_union())
    }
}

/// Result of resolving a designated initializer field name.
#[derive(Debug, Clone)]
pub enum InitFieldResolution {
    /// Found as a direct field at this index.
    Direct(usize),
    /// Found inside an anonymous struct/union member at the given index.
    /// The String is the original designator name to use when drilling into
    /// the anonymous member.
    AnonymousMember {
        anon_field_idx: usize,
        inner_name: String,
    },
}

/// How a field of a record with reverse `scalar_storage_order` must be
/// byte-manipulated at every access. `None` for records with the native
/// storage order (the overwhelmingly common case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SsoMode {
    /// Native order: no byteswap at access.
    #[default]
    None,
    /// The field is stored with its containing storage unit byte-swapped
    /// (big-endian unit on a little-endian host). The `u32` is the unit
    /// size in bytes (2, 4, 8 or 16). Access = load unit, bswap, then apply
    /// the (already reversed) bit offset / plain value semantics.
    /// Used for scalar members and standard (non-packed) bitfields.
    ByteSwapUnit(u32),
    /// Packed bitfield under reverse storage order: the bit stream fills each
    /// BYTE MSB-first. Layout bit offsets stay logical (LSB-first within the
    /// packed run); access = per-byte bit-reverse of the loaded bytes before
    /// extraction (and the inverse after insertion).
    ByteBitReverse,
}

/// Layout info for a single field.
#[derive(Debug, Clone)]
pub struct StructFieldLayout {
    pub name: String,
    pub offset: usize,
    pub ty: CType,
    /// For bitfields: bit offset within the storage unit at `offset`.
    pub bit_offset: Option<u32>,
    /// For bitfields: width in bits.
    pub bit_width: Option<u32>,
    /// Reverse scalar_storage_order handling for this field.
    pub sso: SsoMode,
    /// For reverse-SSO standard bitfields sharing a multi-byte unit: the
    /// unit-wide storage type the access must load (e.g. the unit covering
    /// bits 4..15 of a `short` unit is `short` even when the declared member
    /// is `char c : 1`). Access code substitutes this for `ty` when reading
    /// or writing the field; C-level typing keeps using `ty`.
    pub sso_storage_ty: Option<CType>,
    /// Volatile base type of the field (element or pointee access). Set from the
    /// declaration, so `s.field` and `p->field` accesses are volatile when it is.
    pub base_volatile: bool,
    /// Const on the field's base type (element or innermost pointee).
    pub base_const: bool,
    /// Volatile field object itself (`int *volatile p`).
    pub object_volatile: bool,
    /// Const on the field object itself, independent of its pointee.
    pub object_const: bool,
    /// Optional sibling count member from `__attribute__((counted_by(member)))`.
    pub counted_by: Option<String>,
    /// CVR bits of each pointer level of the field's type (see
    /// [`StructField::pointer_quals`]).
    pub pointer_quals: Vec<u8>,
    /// See [`StructField::own_cvr`].
    pub own_cvr: Option<u8>,
}

impl StructLayout {
    /// Return the smallest integer CType that can hold at least `needed_bytes` bytes.
    /// Preserves signedness.
    /// Uses LongLong (always 8 bytes) instead of Long for >4 bytes, because on
    /// i686 (ILP32) Long is only 4 bytes and would truncate 64-bit bitfields.
    fn smallest_int_ctype_for_bytes(needed_bytes: usize, is_signed: bool) -> CType {
        if is_signed {
            match needed_bytes {
                0..=1 => CType::signed_char(),
                2 => CType::Short,
                3..=4 => CType::Int,
                _ => CType::LongLong,
            }
        } else {
            match needed_bytes {
                0..=1 => CType::UChar,
                2 => CType::UShort,
                3..=4 => CType::UInt,
                _ => CType::ULongLong,
            }
        }
    }

    /// Compute the layout for a struct (fields laid out sequentially with alignment padding).
    /// Supports bitfield packing: adjacent bitfields share storage units.
    /// `max_field_align`: if Some(N), cap each field's alignment to min(natural, N).
    ///   For __attribute__((packed)), pass Some(1).
    ///   For #pragma pack(N), pass Some(N).
    ///   For normal structs, pass None.
    /// `ctx`: provides struct/union layout lookup for nested struct/union field types.
    pub fn for_struct_with_packing(
        fields: &[StructField],
        max_field_align: Option<usize>,
        ctx: &dyn StructLayoutProvider,
        reverse_sso: bool,
    ) -> Self {
        let mut b = StructLayoutBuilder::new(fields.len(), max_field_align, reverse_sso);

        for field in fields {
            let (field_align, field_size) = b.compute_field_alignment(field, max_field_align, ctx);

            if let Some(bw) = field.bit_width {
                if bw == 0 {
                    b.layout_zero_width_bitfield(field_align);
                } else if b.is_packed_1 || field.is_packed {
                    b.layout_packed_bitfield(field, bw, field_size);
                } else {
                    b.layout_standard_bitfield(field, bw, field_size, field_align);
                }
            } else {
                b.layout_regular_field(field, field_size, field_align);
            }
        }

        b.finalize()
    }

    /// Compute the layout for a union with optional packing.
    /// `max_field_align`: if Some(N), cap each field's alignment to min(natural, N).
    ///   For __attribute__((packed)), pass Some(1).
    ///   For #pragma pack(N), pass Some(N).
    ///   For normal unions, pass None.
    pub fn for_union_with_packing(
        fields: &[StructField],
        max_field_align: Option<usize>,
        ctx: &dyn StructLayoutProvider,
        reverse_sso: bool,
    ) -> Self {
        let mut max_size = 0usize;
        let mut max_align = 1usize;
        let mut user_align = false;
        let mut field_layouts = Vec::with_capacity(fields.len());

        for field in fields {
            let natural_align = field.type_align.unwrap_or_else(|| field.ty.align_ctx(ctx));
            // Per-field alignment handling
            let field_align = if field.is_packed {
                // Per-field __attribute__((packed)) forces alignment to 1 unless an
                // explicit aligned(N) / _Alignas(N) attribute also sets floor alignment.
                field.alignment.unwrap_or(1)
            } else if let Some(explicit) = field.alignment {
                // Explicit alignment attribute overrides packing
                natural_align.max(explicit)
            } else if let Some(max_a) = max_field_align {
                natural_align.min(max_a)
            } else {
                natural_align
            };
            let field_size = field.ty.size_ctx(ctx);
            max_align = max_align.max(field_align);
            max_size = max_size.max(field_size);
            user_align |= field_contributes_user_align(field, natural_align, ctx);

            // For union bitfield members, set bit_offset to 0 (all fields start at byte 0)
            // and propagate bit_width so extraction is performed on read.
            // Zero-width bitfields get Some(0) for both bit_offset and bit_width so that
            // resolve_init_field() correctly identifies them as unnamed bitfields to skip
            // during positional initialization (the skip check tests bit_width.is_some()).
            let (bf_offset, bf_width) = if let Some(bw) = field.bit_width {
                (Some(0u32), Some(bw))
            } else {
                (None, None)
            };

            // Reverse SSO in a union: multi-byte scalar members and standard
            // bitfields are byte-reversed within their storage (bit position 0
            // in a union bitfield maps to the TOP of the unit); packed run
            // handling follows the struct builder's convention.
            let sso = if !reverse_sso {
                SsoMode::None
            } else if field.bit_width.is_some() {
                if max_field_align == Some(1) {
                    SsoMode::ByteBitReverse
                } else if field_size >= 2 {
                    SsoMode::ByteSwapUnit(field_size as u32)
                } else {
                    SsoMode::None
                }
            } else if field_size >= 2 {
                SsoMode::ByteSwapUnit(field_size as u32)
            } else {
                SsoMode::None
            };

            // Reverse-SSO union bitfields with multi-byte units need the
            // unit-wide access type; the union bit offset maps to the top of
            // the unit.
            let (bf_offset, sso_storage_ty) =
                if let (Some(bw), Some(bo)) = (field.bit_width, bf_offset) {
                    if reverse_sso && field_size >= 2 && max_field_align != Some(1) {
                        let unit_ctype = StructLayout::smallest_int_ctype_for_bytes(
                            field_size,
                            field.ty.is_signed(),
                        );
                        (Some(field_size as u32 * 8 - bo - bw), Some(unit_ctype))
                    } else if reverse_sso && field_size == 1 {
                        (Some(8 - bo - bw), None)
                    } else {
                        (Some(bo), None)
                    }
                } else {
                    (bf_offset, None)
                };

            field_layouts.push(StructFieldLayout {
                name: field.name.clone(),
                base_volatile: field.base_volatile,
                base_const: field.base_const,
                object_volatile: field.object_volatile,
                object_const: field.object_const,
                counted_by: field.counted_by.clone(),
                pointer_quals: field.pointer_quals.clone(),
                own_cvr: field.own_cvr,
                offset: 0, // All union fields start at offset 0
                ty: field.ty.clone(),
                bit_offset: bf_offset,
                bit_width: bf_width,
                sso,
                sso_storage_ty,
            });
        }

        let size = align_up(max_size, max_align);

        StructLayout {
            is_complete: true,
            fields: field_layouts,
            size,
            align: max_align,
            is_union: true,
            is_transparent_union: false,
            reverse_sso,
            user_align,
        }
    }

    /// Check if any field in this layout is or contains a pointer/function type.
    /// Recursively checks array element types and nested struct/union fields.
    pub fn has_pointer_fields(&self, ctx: &dyn StructLayoutProvider) -> bool {
        self.fields
            .iter()
            .any(|f| Self::field_type_has_pointers(&f.ty, ctx))
    }

    /// Check if a type is or contains pointer/function types (recursive).
    fn field_type_has_pointers(ty: &CType, ctx: &dyn StructLayoutProvider) -> bool {
        match ty {
            CType::Pointer(_, _) | CType::Function(_) => true,
            CType::Array(inner, _) => Self::field_type_has_pointers(inner, ctx),
            CType::Struct(key) | CType::Union(key) => {
                if let Some(layout) = ctx.get_struct_layout(key) {
                    layout
                        .fields
                        .iter()
                        .any(|f| Self::field_type_has_pointers(&f.ty, ctx))
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// SysV AMD64 argument classes of this record (see `sysv_classify`):
    /// register classes, `[Memory]` for a record passed in memory, or an
    /// empty list for records over 16 bytes (MEMORY by size) and empty ones.
    pub fn classify_sysv_eightbytes(&self, ctx: &dyn StructLayoutProvider) -> Vec<EightbyteClass> {
        if self.size > 16 || self.size == 0 {
            return Vec::new();
        }
        super::sysv_classify::classify_layout(self, ctx).arg_classes()
    }

    /// Classify this struct for the RISC-V LP64D hardware floating-point calling convention.
    ///
    /// The RISC-V psABI specifies that small structs (≤ 2×XLEN = 16 bytes on RV64)
    /// with floating-point members should be passed in FP registers:
    /// - Struct with exactly 1 float/double member (no other data): 1 FP register
    /// - Struct with exactly 2 float/double members (no other data): 2 FP registers
    /// - Struct with 1 float/double + 1 integer (≤ XLEN): FP reg + GP reg (or GP + FP)
    /// - All other structs: use integer calling convention (GP registers)
    ///
    /// Returns `None` if the struct does not qualify for FP register passing.
    /// Returns `Some(RiscvFloatClass)` describing the FP register assignment.
    pub fn classify_riscv_float_fields(
        &self,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<RiscvFloatClass> {
        // Only for small structs (≤ 16 bytes on RV64, ≤ 8 bytes on RV32)
        let xlen = target_ptr_size();
        if self.size > 2 * xlen || self.size == 0 {
            return None;
        }

        // Unions cannot be classified as float aggregates per the psABI
        if self.is_union {
            return None;
        }

        // Flatten all scalar fields (recursing into nested structs/arrays)
        let mut float_fields: Vec<(usize, usize)> = Vec::new(); // (offset, size_bytes: 4=float, 8=double)
        let mut int_fields: Vec<(usize, usize)> = Vec::new(); // (offset, size_bytes)
        if !Self::collect_riscv_fields(&self.fields, 0, &mut float_fields, &mut int_fields, ctx) {
            return None;
        }

        // Apply RISC-V psABI rules:
        match (float_fields.len(), int_fields.len()) {
            // 1 float, no int: pass float in 1 FP reg
            (1, 0) => Some(RiscvFloatClass::OneFloat {
                size: float_fields[0].1 as u8,
            }),
            // 2 floats, no int: pass both in FP regs
            (2, 0) => Some(RiscvFloatClass::TwoFloats {
                lo_size: float_fields[0].1 as u8,
                hi_size: float_fields[1].1 as u8,
                hi_offset: float_fields[1].0,
            }),
            // 1 float + 1 int: pass float in FP reg, int in GP reg
            (1, 1) => {
                let float_size = float_fields[0].1 as u8;
                let int_size = int_fields[0].1;
                // Integer field must fit in one XLEN register
                if int_size > xlen {
                    return None;
                }
                // Determine which comes first in memory layout
                if float_fields[0].0 < int_fields[0].0 {
                    Some(RiscvFloatClass::FloatAndInt {
                        float_size,
                        float_offset: float_fields[0].0,
                        int_offset: int_fields[0].0,
                        int_size,
                    })
                } else {
                    Some(RiscvFloatClass::IntAndFloat {
                        float_size,
                        int_offset: int_fields[0].0,
                        int_size,
                        float_offset: float_fields[0].0,
                    })
                }
            }
            _ => None,
        }
    }

    /// Recursively collect scalar float and int fields from a struct.
    /// Returns false if the struct contains unsupported field types (e.g., bitfields,
    /// empty arrays, complex types) that prevent FP classification.
    fn collect_riscv_fields(
        fields: &[StructFieldLayout],
        base_offset: usize,
        float_fields: &mut Vec<(usize, usize)>,
        int_fields: &mut Vec<(usize, usize)>,
        ctx: &dyn StructLayoutProvider,
    ) -> bool {
        for field in fields {
            // Skip zero-width bitfields (padding only)
            if field.bit_width == Some(0) {
                continue;
            }
            // Any bitfield prevents FP classification
            if field.bit_width.is_some() {
                return false;
            }
            let offset = base_offset + field.offset;
            match &field.ty {
                CType::Float16 | CType::BFloat16 => {
                    float_fields.push((offset, 2));
                }
                CType::Float => {
                    float_fields.push((offset, 4));
                }
                CType::Double => {
                    float_fields.push((offset, 8));
                }
                // Nested struct: recurse
                CType::Struct(key) | CType::Union(key) => {
                    if let Some(layout) = ctx.get_struct_layout(key) {
                        if layout.is_union {
                            // Unions break FP classification
                            return false;
                        }
                        if !Self::collect_riscv_fields(
                            &layout.fields,
                            offset,
                            float_fields,
                            int_fields,
                            ctx,
                        ) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                // Array: unroll elements
                CType::Array(elem_ty, Some(count)) => {
                    let elem_size = elem_ty.size();
                    if elem_size == 0 {
                        continue; // zero-size arrays (FAMs) are ignored
                    }
                    for i in 0..*count {
                        let elem_offset = offset + i * elem_size;
                        match elem_ty.as_ref() {
                            CType::Float16 | CType::BFloat16 => float_fields.push((elem_offset, 2)),
                            CType::Float => float_fields.push((elem_offset, 4)),
                            CType::Double => float_fields.push((elem_offset, 8)),
                            CType::Struct(key) | CType::Union(key) => {
                                if let Some(layout) = ctx.get_struct_layout(key) {
                                    if layout.is_union {
                                        return false;
                                    }
                                    if !Self::collect_riscv_fields(
                                        &layout.fields,
                                        elem_offset,
                                        float_fields,
                                        int_fields,
                                        ctx,
                                    ) {
                                        return false;
                                    }
                                } else {
                                    return false;
                                }
                            }
                            _ => {
                                let size = elem_ty.size();
                                if size > 0 {
                                    int_fields.push((elem_offset, size));
                                }
                            }
                        }
                    }
                }
                // Zero-size arrays or VLAs don't contribute
                CType::Array(_, None) => {}
                // All other scalar types: integer-class
                _ => {
                    let size = field.ty.size();
                    if size > 0 {
                        int_fields.push((offset, size));
                    }
                }
            }

            // If we already have more than 2 fields total, can't classify as FP
            if float_fields.len() + int_fields.len() > 2 {
                return false;
            }
            // If we have more than 1 int field, can't classify as FP
            if int_fields.len() > 1 {
                return false;
            }
        }
        true
    }

    /// Resolve which field index an initializer targets, given either a field designator
    /// or a positional index that skips unnamed bitfield fields.
    ///
    /// `designator_name`: If Some, look up the field by name.
    /// `current_idx`: The current positional index (used when no designator).
    ///
    /// Per C11 6.7.9p9, unnamed members of structure types do not participate in
    /// initialization. Anonymous struct/union members (empty name, no bit_width) DO
    /// participate. Unnamed bitfields (empty name, has bit_width) do NOT.
    ///
    /// Returns the resolved field index, or `None` if no valid field found.
    pub fn resolve_init_field_idx(
        &self,
        designator_name: Option<&str>,
        current_idx: usize,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<usize> {
        match self.resolve_init_field(designator_name, current_idx, ctx) {
            Some(InitFieldResolution::Direct(idx)) => Some(idx),
            Some(InitFieldResolution::AnonymousMember { anon_field_idx, .. }) => {
                Some(anon_field_idx)
            }
            None => None,
        }
    }

    /// Resolve which field an initializer targets, with full info about anonymous members.
    ///
    /// When a designator name is found inside an anonymous struct/union member,
    /// returns `AnonymousMember` with the anonymous field's index and the inner name,
    /// allowing callers to drill into the anonymous member for proper initialization.
    pub fn resolve_init_field(
        &self,
        designator_name: Option<&str>,
        current_idx: usize,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<InitFieldResolution> {
        if let Some(name) = designator_name {
            // First try direct field lookup
            if let Some(idx) = self.fields.iter().position(|f| f.name == name) {
                return Some(InitFieldResolution::Direct(idx));
            }
            // Search inside anonymous struct/union members
            for (idx, f) in self.fields.iter().enumerate() {
                // Anonymous members have empty name and no bit_width
                if !f.name.is_empty() || f.bit_width.is_some() {
                    continue;
                }
                match &f.ty {
                    CType::Struct(key) | CType::Union(key) => {
                        if Self::anon_member_contains_field_ctx(key, name, ctx) {
                            return Some(InitFieldResolution::AnonymousMember {
                                anon_field_idx: idx,
                                inner_name: name.to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            None
        } else {
            // Positional init: skip unnamed bitfields (empty name + has bit_width).
            // Anonymous struct/union members (empty name, no bit_width) still participate.
            let mut idx = current_idx;
            while idx < self.fields.len() {
                let f = &self.fields[idx];
                if f.name.is_empty() && f.bit_width.is_some() {
                    // Unnamed bitfield: skip it
                    idx += 1;
                } else {
                    return Some(InitFieldResolution::Direct(idx));
                }
            }
            None
        }
    }

    /// Check if an anonymous struct/union member (identified by layout key) contains
    /// a field with the given name, including recursively through nested anonymous members.
    fn anon_member_contains_field_ctx(
        key: &str,
        name: &str,
        ctx: &dyn StructLayoutProvider,
    ) -> bool {
        if let Some(layout) = ctx.get_struct_layout(key) {
            for f in &layout.fields {
                if f.name == name {
                    return true;
                }
                // Recurse into nested anonymous members
                if f.name.is_empty() {
                    match &f.ty {
                        CType::Struct(inner_key) | CType::Union(inner_key) => {
                            if Self::anon_member_contains_field_ctx(inner_key, name, ctx) {
                                return true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }

    /// Look up a field by name, returning its offset and a clone of its type.
    /// Recursively searches anonymous struct/union members.
    pub fn field_offset(
        &self,
        name: &str,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<(usize, CType)> {
        // First, try direct field lookup
        if let Some(f) = self.fields.iter().find(|f| f.name == name) {
            return Some((f.offset, f.ty.clone()));
        }
        // Then, search anonymous (unnamed) struct/union members recursively
        for f in &self.fields {
            if !f.name.is_empty() {
                continue;
            }
            let anon_key = match &f.ty {
                CType::Struct(key) | CType::Union(key) => key.clone(),
                _ => continue,
            };
            let anon_layout = match ctx.get_struct_layout(&anon_key) {
                Some(layout) => layout.clone(),
                None => continue,
            };
            // Check if the target field is directly in this anonymous member
            if let Some(inner_field) = anon_layout.fields.iter().find(|sf| sf.name == name) {
                // Compute offset within the anonymous struct/union
                let inner_offset = match &f.ty {
                    CType::Struct(_) => anon_layout
                        .field_offset(name, ctx)
                        .map(|(o, _)| o)
                        .unwrap_or(0),
                    CType::Union(_) => 0, // all union fields at offset 0
                    _ => 0,
                };
                return Some((f.offset + inner_offset, inner_field.ty.clone()));
            }
            // Recurse into nested anonymous members
            if let Some((inner_offset, ty)) = anon_layout.field_offset(name, ctx) {
                return Some((f.offset + inner_offset, ty));
            }
        }
        None
    }

    /// The complete own CVR bits of member `name` (see
    /// [`StructField::own_cvr`]), searched like [`Self::field_offset`]: a
    /// member reached through an anonymous one also carries that member's
    /// qualifiers (`struct { const struct { int x; }; } s;` makes `s.x`
    /// const). `None` when any link is unknown.
    pub fn member_own_cvr(&self, name: &str, ctx: &dyn StructLayoutProvider) -> Option<u8> {
        if let Some(f) = self.fields.iter().find(|f| f.name == name) {
            return f.own_cvr;
        }
        for f in self.fields.iter().filter(|f| f.name.is_empty()) {
            let (CType::Struct(key) | CType::Union(key)) = &f.ty else {
                continue;
            };
            let Some(anon) = ctx.get_struct_layout(key) else {
                continue;
            };
            if anon.field_offset(name, ctx).is_some() {
                return Some(f.own_cvr? | anon.member_own_cvr(name, ctx)?);
            }
        }
        None
    }

    /// Look up a field by name, returning full layout info including bitfield details.
    pub fn field_layout(&self, name: &str) -> Option<&StructFieldLayout> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Resolve a field's `counted_by` attribute, returning
    /// `(count_member_name, count_member_ctype, fam_field_ctype)` when the
    /// named field carries `counted_by` and its sibling count field exists.
    pub fn field_counted_by(
        &self,
        name: &str,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<(String, CType, CType)> {
        if let Some(f) = self.fields.iter().find(|f| f.name == name) {
            let cnt_name = f.counted_by.as_ref()?;
            let (_, cnt_ty) = self.field_offset(cnt_name, ctx)?;
            return Some((cnt_name.clone(), cnt_ty, f.ty.clone()));
        }
        for f in &self.fields {
            if !f.name.is_empty() {
                continue;
            }
            if let CType::Struct(key) | CType::Union(key) = &f.ty {
                if let Some(anon) = ctx.get_struct_layout(key) {
                    if let Some((cnt_name, _, fam_ty)) = anon.field_counted_by(name, ctx) {
                        if let Some((_, cnt_ty)) = self.field_offset(&cnt_name, ctx) {
                            return Some((cnt_name, cnt_ty, fam_ty));
                        }
                    }
                }
            }
        }
        None
    }

    /// Look up a field's volatile qualifiers and type, searching anonymous
    /// struct/union members recursively (like `field_offset_with_bitfield`).
    /// Returns `(base_volatile, object_volatile, ty)`.
    pub fn field_volatility(
        &self,
        name: &str,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<(bool, bool, CType)> {
        if let Some(f) = self.fields.iter().find(|f| f.name == name) {
            return Some((f.base_volatile, f.object_volatile, f.ty.clone()));
        }
        for f in &self.fields {
            if !f.name.is_empty() {
                continue;
            }
            let anon_key = match &f.ty {
                CType::Struct(key) | CType::Union(key) => key.clone(),
                _ => continue,
            };
            let anon_layout = match ctx.get_struct_layout(&anon_key) {
                Some(layout) => layout.clone(),
                None => continue,
            };
            if let Some(found) = anon_layout.field_volatility(name, ctx) {
                return Some(found);
            }
        }
        None
    }

    /// Look up a field's const qualifiers and type, searching anonymous
    /// struct/union members recursively (like `field_offset_with_bitfield`).
    /// Returns `(base_const, object_const, ty)`.
    pub fn field_constness(
        &self,
        name: &str,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<(bool, bool, CType)> {
        if let Some(f) = self.fields.iter().find(|f| f.name == name) {
            return Some((f.base_const, f.object_const, f.ty.clone()));
        }
        for f in &self.fields {
            if !f.name.is_empty() {
                continue;
            }
            let anon_key = match &f.ty {
                CType::Struct(key) | CType::Union(key) => key.clone(),
                _ => continue,
            };
            let anon_layout = match ctx.get_struct_layout(&anon_key) {
                Some(layout) => layout.clone(),
                None => continue,
            };
            if let Some(found) = anon_layout.field_constness(name, ctx) {
                return Some((found.0, found.1 || f.object_const, found.2));
            }
        }
        None
    }

    /// Per-level CVR bits of member `name` (see [`StructField::pointer_quals`]),
    /// searched through anonymous members. `None` when the member is unknown or
    /// carried no pointer-level qualifier.
    pub fn field_pointer_quals(
        &self,
        name: &str,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<Vec<u8>> {
        if let Some(f) = self.fields.iter().find(|f| f.name == name) {
            return (!f.pointer_quals.is_empty()).then(|| f.pointer_quals.clone());
        }
        for f in &self.fields {
            if !f.name.is_empty() {
                continue;
            }
            let anon_key = match &f.ty {
                CType::Struct(key) | CType::Union(key) => key.clone(),
                _ => continue,
            };
            if let Some(found) = ctx
                .get_struct_layout(&anon_key)
                .and_then(|layout| layout.field_pointer_quals(name, ctx))
            {
                return Some(found);
            }
        }
        None
    }

    /// Look up a field by name, returning its offset, type, and optional bitfield info.
    /// Recursively searches anonymous struct/union members (unlike field_layout which
    /// only does flat lookup). This is needed so that bitfield metadata is not lost
    /// when accessing bitfields through anonymous struct/union members.
    pub fn field_offset_with_bitfield(
        &self,
        name: &str,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<(usize, CType, Option<u32>, Option<u32>)> {
        // First, try direct field lookup
        if let Some(f) = self.fields.iter().find(|f| f.name == name) {
            return Some((f.offset, f.ty.clone(), f.bit_offset, f.bit_width));
        }
        // Then, search anonymous (unnamed) struct/union members recursively
        for f in &self.fields {
            if !f.name.is_empty() {
                continue;
            }
            let anon_key = match &f.ty {
                CType::Struct(key) | CType::Union(key) => key.clone(),
                _ => continue,
            };
            let anon_layout = match ctx.get_struct_layout(&anon_key) {
                Some(layout) => layout.clone(),
                None => continue,
            };
            if let Some((inner_offset, ty, bit_offset, bit_width)) =
                anon_layout.field_offset_with_bitfield(name, ctx)
            {
                return Some((f.offset + inner_offset, ty, bit_offset, bit_width));
            }
        }
        None
    }
}

/// Align `offset` up to the next multiple of `align`.
pub fn align_up(offset: usize, align: usize) -> usize {
    if align == 0 {
        return offset;
    }
    let mask = align - 1;
    match offset.checked_add(mask) {
        Some(v) => v & !mask,
        None => offset, // overflow: return offset unchanged
    }
}

impl CType {
    /// The user-visible tag of a record type key.
    ///
    /// Strips the internal `struct.`/`union.` prefix and the `#N` suffix that
    /// marks a shadowing definition (C 6.7.2.3: an inner-scope `struct S` is a
    /// distinct type from an outer `struct S`, so the two need distinct keys
    /// while still printing identically). A C identifier cannot contain `#`, so
    /// cutting there is unambiguous.
    pub fn record_display_tag(key: &str) -> &str {
        let without_prefix = key
            .strip_prefix("union.")
            .or_else(|| key.strip_prefix("struct."))
            .unwrap_or(key);
        match without_prefix.find('#') {
            Some(i) => &without_prefix[..i],
            None => without_prefix,
        }
    }
}

/// Whether `ty` must be spelled with C's abstract declarator syntax: its
/// pointer/array chain holds an array that is pointed to or nested in another
/// array (`int (*)[3]`, `int[3][4]`), and nothing the plain spelling handles
/// specially (a function, a named address space).
fn needs_declarator_syntax(ty: &CType) -> bool {
    let mut cur = ty;
    let mut derived_above = false;
    let mut found = false;
    loop {
        match cur {
            CType::Pointer(inner, quals) => {
                if quals.space != AddressSpace::Default {
                    return false;
                }
                cur = inner;
            }
            CType::Array(inner, _) => {
                found |= derived_above;
                cur = inner;
            }
            CType::Function(_) => return false,
            _ => return found,
        }
        derived_above = true;
    }
}

/// GCC's spelling of a [`needs_declarator_syntax`] type, built inside out
/// (measured on gcc-16.2): `int (*)[3]`, `int (**)[3]`, `const int (*)[3]`,
/// `int (* const*)[3]`, `int * (*)[3]`, `char * const (*)[2]`,
/// `int (*)[3][4]`, `int[3][4]`. A pointer level's recorded qualifiers follow
/// its `*`; the base's lead; an array is qualified like its elements.
fn write_declarator_type(f: &mut std::fmt::Formatter<'_>, ty: &CType) -> std::fmt::Result {
    use std::fmt::Write as _;
    let mut declarator = String::new();
    // Qualifiers of the object of type `cur`: unknown (none) for the value
    // itself, then each pointer's record of its pointee.
    let mut own = 0u8;
    let mut cur = ty;
    loop {
        match cur {
            CType::Pointer(inner, quals) => {
                let gap = if declarator.starts_with('(') { " " } else { "" };
                declarator = if own != 0 {
                    format!("* {}{gap}{declarator}", cvr_spelling(own))
                } else {
                    format!("*{gap}{declarator}")
                };
                own = quals.cvr().unwrap_or(0);
                cur = inner;
            }
            CType::Array(inner, size) => {
                if declarator.starts_with('*') {
                    declarator = format!("({declarator})");
                }
                match size {
                    Some(n) => write!(declarator, "[{n}]")?,
                    None => declarator.push_str("[]"),
                }
                cur = inner;
            }
            leaf => {
                if own != 0 {
                    write!(f, "{} ", cvr_spelling(own))?;
                }
                let gap = if declarator.starts_with('[') { "" } else { " " };
                return write!(f, "{leaf}{gap}{declarator}");
            }
        }
    }
}

impl std::fmt::Display for CType {
    /// Format a CType as its C-language type name (e.g., `int`, `unsigned long`,
    /// `char *`, `void (*)(int, double)`). Used in compiler diagnostics to show
    /// user-friendly type names instead of the Rust Debug representation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if needs_declarator_syntax(self) {
            return write_declarator_type(f, self);
        }
        match self {
            CType::Void => write!(f, "void"),
            CType::NullPtr => write!(f, "nullptr_t"),
            CType::Bool => write!(f, "_Bool"),
            CType::Char if char_is_unsigned() => write!(f, "signed char"),
            CType::Char | CType::PlainUChar => write!(f, "char"),
            CType::SChar => write!(f, "signed char"),
            CType::UChar => write!(f, "unsigned char"),
            CType::Short => write!(f, "short"),
            CType::UShort => write!(f, "unsigned short"),
            CType::Int => write!(f, "int"),
            CType::UInt => write!(f, "unsigned int"),
            CType::Long => write!(f, "long"),
            CType::ULong => write!(f, "unsigned long"),
            CType::LongLong => write!(f, "long long"),
            CType::ULongLong => write!(f, "unsigned long long"),
            CType::Int128 => write!(f, "__int128"),
            CType::UInt128 => write!(f, "unsigned __int128"),
            CType::Float => write!(f, "float"),
            CType::Double => write!(f, "double"),
            CType::LongDouble => write!(f, "long double"),
            CType::Float16 => write!(f, "_Float16"),
            CType::BFloat16 => write!(f, "__bf16"),
            CType::Float128 => write!(f, "_Float128"),
            CType::Decimal32 => write!(f, "_Decimal32"),
            CType::Decimal64 => write!(f, "_Decimal64"),
            CType::Decimal128 => write!(f, "_Decimal128"),
            CType::ComplexFloat => write!(f, "_Complex float"),
            CType::ComplexDouble => write!(f, "_Complex double"),
            CType::ComplexLongDouble => write!(f, "_Complex long double"),
            CType::Pointer(inner, addr_space) => {
                let prefix = match addr_space.space {
                    AddressSpace::Default => "",
                    AddressSpace::SegGs => "__seg_gs ",
                    AddressSpace::SegFs => "__seg_fs ",
                };
                // Function pointer: void (*)(int) instead of void (*)(int) *
                if let CType::Function(ft) = inner.as_ref() {
                    write!(f, "{}", ft.return_type)?;
                    write!(f, " ({}*)(", prefix)?;
                    for (i, (param_ty, _)) in ft.params.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", param_ty)?;
                    }
                    if ft.params.is_empty() && ft.has_prototype && !ft.variadic {
                        write!(f, "void")?;
                    }
                    if ft.variadic {
                        if !ft.params.is_empty() {
                            write!(f, ", ")?;
                        }
                        write!(f, "...")?;
                    }
                    write!(f, ")")
                } else {
                    // GCC's spelling: the base type's qualifiers lead
                    // (`const char *`), a pointer level's precede its own `*`
                    // (`const char * const*`, `char **`).
                    let cvr = addr_space.cvr().unwrap_or(0);
                    match inner.as_ref() {
                        CType::Pointer(next, _) if !matches!(next.as_ref(), CType::Function(_)) => {
                            if cvr != 0 {
                                write!(f, "{}{} {}*", prefix, inner, cvr_spelling(cvr))
                            } else {
                                write!(f, "{}{}*", prefix, inner)
                            }
                        }
                        _ if cvr != 0 => write!(f, "{} {}{} *", cvr_spelling(cvr), prefix, inner),
                        _ => write!(f, "{}{} *", prefix, inner),
                    }
                }
            }
            CType::Array(inner, size) => {
                if let Some(n) = size {
                    write!(f, "{}[{}]", inner, n)
                } else {
                    write!(f, "{}[]", inner)
                }
            }
            CType::Function(ft) => {
                write!(f, "{} (", ft.return_type)?;
                for (i, (param_ty, _)) in ft.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param_ty)?;
                }
                if ft.params.is_empty() && ft.has_prototype && !ft.variadic {
                    write!(f, "void")?;
                }
                if ft.variadic {
                    if !ft.params.is_empty() {
                        write!(f, ", ")?;
                    }
                    write!(f, "...")?;
                }
                write!(f, ")")
            }
            CType::Struct(name) => {
                let display_name = CType::record_display_tag(name);
                if display_name.starts_with("__anon_struct_") {
                    write!(f, "struct <anonymous>")
                } else {
                    write!(f, "struct {}", display_name)
                }
            }
            CType::Union(name) => {
                let display_name = CType::record_display_tag(name);
                if display_name.starts_with("__anon_struct_") {
                    write!(f, "union <anonymous>")
                } else {
                    write!(f, "union {}", display_name)
                }
            }
            CType::Enum(e) => {
                if let Some(ref name) = e.name {
                    write!(f, "enum {}", name)
                } else {
                    write!(f, "enum <anonymous>")
                }
            }
            CType::Vector(elem, total_size) => {
                // GCC-style vector type display
                write!(f, "__attribute__((vector_size({}))) {}", total_size, elem)
            }
            CType::ExtVector(elem, layout) => {
                // Clang's canonical spelling (the element first); Clang shows
                // a typedef name instead where one was written, which a
                // CType does not record.
                write!(
                    f,
                    "{} __attribute__((ext_vector_type({})))",
                    elem, layout.lanes
                )
            }
        }
    }
}

/// Whether the target's plain `char` is unsigned (AArch64 and RISC-V SysV
/// ABIs specify unsigned char; x86-64/i686 use signed char). One target per
/// driver process, so a process-global set by the driver before parsing is
/// the single source of truth — the front end is otherwise target-blind.
static CHAR_UNSIGNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record the target's plain-char signedness. Called once by the driver
/// after the target is known, before any parsing/sema/lowering runs.
pub fn set_char_unsigned(unsigned: bool) {
    CHAR_UNSIGNED.store(unsigned, std::sync::atomic::Ordering::Relaxed);
}

/// The target's plain-char signedness (true: `char` behaves as `unsigned char`).
impl CType {
    /// The type of plain `char` on the current target.
    pub fn plain_char() -> CType {
        if char_is_unsigned() {
            CType::PlainUChar
        } else {
            CType::Char
        }
    }

    /// The type of explicit `signed char` on the current target.
    pub fn signed_char() -> CType {
        if char_is_unsigned() {
            CType::Char
        } else {
            CType::SChar
        }
    }

    /// Whether this is one of the three character types (C11 6.2.5p15).
    pub fn is_char_type(&self) -> bool {
        matches!(
            self,
            CType::Char | CType::SChar | CType::UChar | CType::PlainUChar
        )
    }
}

pub fn char_is_unsigned() -> bool {
    CHAR_UNSIGNED.load(std::sync::atomic::Ordering::Relaxed)
}

/// `-fshort-wchar`: `wchar_t` is a 16-bit unsigned type instead of a 32-bit
/// signed one.
///
/// This is not a cosmetic width knob — it changes the type of every `L"..."`
/// literal, so it has to be visible to the parser, sema, sizeof, array
/// initialisation and the backend alike. It is recorded process-globally for
/// the same reason plain-char signedness is: the whole front end is otherwise
/// target-blind and there is exactly one language definition per driver
/// process. The driver installs it while parsing arguments, before any input
/// file is read.
///
/// The Linux kernel builds with `-fshort-wchar` (its `wchar_t` must match the
/// 16-bit `CHAR16` used by EFI: `include/linux/efi.h` passes `L"SecureBoot"`
/// to a function taking `unsigned short *`), so honouring the flag is a hard
/// requirement for kernel compilation, not an optional GCC compatibility.
static SHORT_WCHAR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record `-fshort-wchar` / `-fno-short-wchar`. Called by the driver before
/// any parsing/sema/lowering runs.
pub fn set_short_wchar(short: bool) {
    SHORT_WCHAR.store(short, std::sync::atomic::Ordering::Relaxed);
}

/// Whether `wchar_t` is the 16-bit `-fshort-wchar` type.
pub fn short_wchar() -> bool {
    SHORT_WCHAR.load(std::sync::atomic::Ordering::Relaxed)
}

/// The C type of `wchar_t` for the current options.
///
/// GCC's `__WCHAR_TYPE__` is `int` by default and `short unsigned int` under
/// `-fshort-wchar` (its `__WCHAR_MIN__`/`__WCHAR_MAX__` follow: 0/0xffff).
/// Every place that needs to name `wchar_t` goes through here so the two can
/// never disagree.
pub fn wchar_ctype() -> CType {
    if short_wchar() {
        CType::UShort
    } else {
        CType::Int
    }
}

/// Spelling of `wchar_t` for `__WCHAR_TYPE__` (GCC: `int` / `short unsigned int`).
pub fn wchar_type_spelling() -> &'static str {
    if short_wchar() {
        "short unsigned int"
    } else {
        "int"
    }
}

/// `__WCHAR_MAX__` (GCC: 0xffff / 0x7fffffff).
pub fn wchar_max() -> &'static str {
    if short_wchar() {
        "0xffff"
    } else {
        "2147483647"
    }
}

/// `__WCHAR_MIN__` (GCC writes the width-dependent spelling shown).
pub fn wchar_min() -> &'static str {
    if short_wchar() {
        "0"
    } else {
        "(-__WCHAR_MAX__ - 1)"
    }
}

/// `__WCHAR_WIDTH__`.
pub fn wchar_width() -> &'static str {
    if short_wchar() { "16" } else { "32" }
}

/// `__SIZEOF_WCHAR_T__`.
pub fn wchar_sizeof() -> &'static str {
    if short_wchar() { "2" } else { "4" }
}

impl CType {
    /// Size in bytes, with struct/union layout lookup via context.
    /// Uses the thread-local target pointer size for target-dependent types
    /// (Long, Pointer, LongDouble).
    pub fn size_ctx(&self, ctx: &dyn StructLayoutProvider) -> usize {
        let ptr_sz = target_ptr_size();
        match self {
            // GCC's documented extension: `sizeof(void)` is 1, and so is
            // `sizeof` of any expression whose value has void type
            // (`sizeof((void)0)`, `sizeof(*void_ptr)` -- the latter is also
            // handled at the deref, for the same reason). Alignment already
            // reports 1 for void, and void * arithmetic steps one byte, so the
            // three agree; 0 stays reserved for the types that genuinely have
            // no size yet (linked structs/unions whose layout is not computed),
            // which must NOT read as 1.
            CType::Void => 1,
            CType::Bool | CType::Char | CType::SChar | CType::UChar | CType::PlainUChar => 1,
            CType::Short | CType::UShort => 2,
            CType::Int | CType::UInt => 4,
            // ILP32: long = 4 bytes; LP64: long = 8 bytes
            CType::Long | CType::ULong => ptr_sz,
            CType::LongLong | CType::ULongLong => 8,
            CType::Int128 | CType::UInt128 => 16,
            CType::Float => 4,
            CType::Double => 8,
            // i686: long double is 12 bytes (80-bit x87 with 4 bytes padding)
            // x86-64: long double is 16 bytes (80-bit x87 with 6 bytes padding)
            // ARM/RISC-V: long double is 16 bytes (IEEE binary128)
            CType::LongDouble => {
                if ptr_sz == 4 {
                    12
                } else {
                    16
                }
            }
            CType::Float16 | CType::BFloat16 => 2,
            CType::Float128 => 16,
            // Decimal floating point (BID): same sizes as the binary types of
            // equal width (GCC: sizeof = 4/8/16 on both x86-64 and i386).
            CType::Decimal32 => 4,
            CType::Decimal64 => 8,
            CType::Decimal128 => 16,
            CType::ComplexFloat => 8,   // 2 * sizeof(float)
            CType::ComplexDouble => 16, // 2 * sizeof(double)
            CType::ComplexLongDouble => {
                if ptr_sz == 4 {
                    24
                } else {
                    32
                }
            }
            CType::Pointer(_, _) | CType::NullPtr => ptr_sz,
            // Saturation, not wrapping: a length that overflows `usize`
            // must only ever make the object *larger* than it can be, never
            // smaller, so that no path can silently produce an under-sized
            // object. Sema rejects the construct before it gets here (a
            // constant bound whose byte size exceeds `max_object_bytes()`),
            // and the assembler rejects the saturated count as a backstop.
            CType::Array(elem, Some(n)) => elem.size_ctx(ctx).saturating_mul(*n),
            CType::Array(_, None) => ptr_sz, // incomplete array treated as pointer
            CType::Function(_) => ptr_sz,    // function pointer size
            CType::Struct(key) | CType::Union(key) => {
                ctx.get_struct_layout(key).map(|l| l.size).unwrap_or(0)
            }
            CType::Enum(e) => e.packed_size(),
            CType::Vector(_, total_size) => *total_size,
            CType::ExtVector(_, layout) => layout.storage_size,
        }
    }

    /// Alignment in bytes, with struct/union layout lookup via context.
    /// Uses the thread-local target pointer size for target-dependent types.
    pub fn align_ctx(&self, ctx: &dyn StructLayoutProvider) -> usize {
        let ptr_sz = target_ptr_size();
        match self {
            CType::Void => 1,
            CType::Bool | CType::Char | CType::SChar | CType::UChar | CType::PlainUChar => 1,
            CType::Short | CType::UShort => 2,
            CType::Int | CType::UInt => 4,
            // ILP32: long aligned to 4; LP64: long aligned to 8
            CType::Long | CType::ULong => ptr_sz,
            // i686: long long aligned to 4 (not 8!) per i386 System V ABI
            // LP64: long long aligned to 8
            CType::LongLong | CType::ULongLong => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            CType::Int128 | CType::UInt128 => {
                if ptr_sz == 4 {
                    4
                } else {
                    16
                }
            }
            CType::Float => 4,
            CType::Double => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            // i686: long double aligned to 4; LP64: aligned to 16
            CType::LongDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    16
                }
            }
            CType::Float16 | CType::BFloat16 => 2,
            CType::Float128 => 16,
            // GCC aligns _Decimal64 to 8 and _Decimal128 to 16 on both
            // x86-64 and i386 (measured: alignof = 4/8/16 for D32/D64/D128).
            CType::Decimal32 => 4,
            CType::Decimal64 => 8,
            CType::Decimal128 => 16,
            CType::ComplexFloat => 4, // align of float component
            CType::ComplexDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    8
                }
            }
            CType::ComplexLongDouble => {
                if ptr_sz == 4 {
                    4
                } else {
                    16
                }
            }
            CType::Pointer(_, _) | CType::NullPtr => ptr_sz,
            CType::Array(elem, _) => elem.align_ctx(ctx),
            CType::Function(_) => ptr_sz,
            CType::Struct(key) | CType::Union(key) => {
                ctx.get_struct_layout(key).map(|l| l.align).unwrap_or(1)
            }
            // Aligned as its compatible type: an 8-byte enum is 4-aligned on
            // i386 like `long long` (GCC 16.2 `_Alignof`, -m32).
            CType::Enum(e) => e.compatible_ctype().align_ctx(ctx),
            // GCC TYPE_ALIGN of a vector is its full size on x86 (struct
            // layout, parameter passing, variable alignment, __alignof__);
            // the 16-byte figure C11 _Alignof reports is a separate cap
            // (`c11_alignof_ctx`), not the type's alignment.
            CType::Vector(_, total_size) => vector_type_align(*total_size),
            // Clang ext_vector_type alignment follows its padded storage size.
            CType::ExtVector(_, layout) => layout.alignment,
        }
    }

    /// GCC `TYPE_USER_ALIGN`: whether this type's alignment was raised by an
    /// explicit request somewhere inside it (record attribute, `_Alignas`
    /// field, nested such record, array of one). Typedef-level `aligned`
    /// overrides are not part of `CType` and are handled by the callers that
    /// consult `typedef_alignments`.
    pub fn has_user_align(&self, ctx: &dyn StructLayoutProvider) -> bool {
        match self {
            CType::Array(elem, _) => elem.has_user_align(ctx),
            CType::Struct(key) | CType::Union(key) => {
                ctx.get_struct_layout(key).is_some_and(|l| l.user_align)
            }
            _ => false,
        }
    }

    /// C11 `_Alignof(type)` — GCC's `min_align_of_type`: the type's
    /// alignment, capped at `BIGGEST_ALIGNMENT` unless the type is
    /// user-aligned. The cap only ever bites for GCC `vector_size` types
    /// wider than the enabled vector ISA (and records/arrays containing one):
    /// `_Alignof(v32)` is 16 by default, 32 under `-mavx`; `__alignof__(v32)`
    /// and the struct-layout/ABI alignment stay 32 (GCC 16.2, -m64 and -m32).
    /// Clang `ext_vector_type` types are not capped (Clang has no such rule
    /// and GCC has no such type), nor is `_Alignof(expr)` — GCC's
    /// `c_alignof_expr` reads the uncapped `DECL_ALIGN`/`TYPE_ALIGN`.
    pub fn c11_alignof_ctx(&self, ctx: &dyn StructLayoutProvider) -> usize {
        let align = self.align_ctx(ctx);
        let cap = target_biggest_alignment();
        if align <= cap || !self.contains_gnu_vector(ctx) || self.has_user_align(ctx) {
            align
        } else {
            cap
        }
    }

    /// Whether a GCC `vector_size` type occurs inside this type (directly, as
    /// an array element, or as a record member at any depth).
    fn contains_gnu_vector(&self, ctx: &dyn StructLayoutProvider) -> bool {
        match self {
            CType::Vector(..) => true,
            CType::Array(elem, _) => elem.contains_gnu_vector(ctx),
            CType::Struct(key) | CType::Union(key) => ctx
                .get_struct_layout(key)
                .is_some_and(|l| l.fields.iter().any(|f| f.ty.contains_gnu_vector(ctx))),
            _ => false,
        }
    }

    /// Preferred (natural) alignment in bytes, as returned by GCC's __alignof__.
    /// On i686, this differs from align_ctx() for long long (8 vs 4) and double (8 vs 4).
    /// On LP64 targets, preferred == ABI alignment, so this returns the same as align_ctx().
    pub fn preferred_align_ctx(&self, ctx: &dyn StructLayoutProvider) -> usize {
        let ptr_sz = target_ptr_size();
        if ptr_sz != 4 {
            // On 64-bit targets, preferred == ABI alignment
            return self.align_ctx(ctx);
        }
        // On i686: long long and double have preferred alignment of 8
        match self {
            CType::Enum(e) => e.compatible_ctype().preferred_align_ctx(ctx),
            CType::LongLong | CType::ULongLong => 8,
            CType::Double => 8,
            CType::ComplexDouble => 8,
            _ => self.align_ctx(ctx),
        }
    }

    /// Size in bytes for non-struct/union types. For struct/union, returns 0.
    /// Use size_ctx() when you need accurate struct/union sizes.
    pub fn size(&self) -> usize {
        match self {
            CType::Struct(_) | CType::Union(_) => 0,
            _ => {
                // For non-struct/union types, we can use an empty provider
                let empty: FxHashMap<String, RcLayout> = FxHashMap::default();
                self.size_ctx(&empty)
            }
        }
    }

    /// GCC's `__builtin_classify_type` class of a value of this type (its
    /// `enum type_class`), after the argument's array and function
    /// decay. Measured with GCC 16.2: `char`, `_Bool` and enums are
    /// `integer_type_class` (1) in C; vectors are `vector_type_class` (19).
    pub fn gcc_type_class(&self) -> i64 {
        match self {
            CType::Void => 0,
            CType::Bool
            | CType::Char
            | CType::SChar
            | CType::UChar
            | CType::PlainUChar
            | CType::Short
            | CType::UShort
            | CType::Int
            | CType::UInt
            | CType::Long
            | CType::ULong
            | CType::LongLong
            | CType::ULongLong
            | CType::Int128
            | CType::UInt128
            | CType::Enum(_) => 1,
            CType::Float
            | CType::Double
            | CType::LongDouble
            | CType::Float16
            | CType::BFloat16
            | CType::Float128
            | CType::Decimal32
            | CType::Decimal64
            | CType::Decimal128 => 8,
            CType::ComplexFloat | CType::ComplexDouble | CType::ComplexLongDouble => 9,
            CType::Pointer(..) | CType::Array(..) | CType::Function(_) => 5,
            // GCC 16.2: `__builtin_classify_type(nullptr)` is -1 (no class).
            CType::NullPtr => -1,
            CType::Struct(_) => 12,
            CType::Union(_) => 13,
            CType::Vector(..) | CType::ExtVector(..) => 19,
        }
    }

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            CType::Bool
                | CType::Char
                | CType::SChar
                | CType::UChar
                | CType::PlainUChar
                | CType::Short
                | CType::UShort
                | CType::Int
                | CType::UInt
                | CType::Long
                | CType::ULong
                | CType::LongLong
                | CType::ULongLong
                | CType::Int128
                | CType::UInt128
                | CType::Enum(_)
        )
    }

    pub fn is_signed(&self) -> bool {
        if let CType::Enum(e) = self {
            return e.compatible_ctype().is_signed();
        }
        matches!(
            self,
            CType::Char
                | CType::SChar
                | CType::Short
                | CType::Int
                | CType::Long
                | CType::LongLong
                | CType::Int128
        )
    }

    /// Whether this is a complex type (_Complex float/double/long double).
    pub fn is_complex(&self) -> bool {
        matches!(
            self,
            CType::ComplexFloat | CType::ComplexDouble | CType::ComplexLongDouble
        )
    }

    /// Whether this is a floating-point type (float, double, long double).
    pub fn is_floating(&self) -> bool {
        matches!(self, CType::Float | CType::Double | CType::LongDouble)
    }

    /// Whether this is a decimal floating-point type (C23 _DecimalN).
    /// Deliberately excluded from is_floating(): decimal types never mix
    /// with binary floats in the usual arithmetic conversions and never
    /// reach binary FP codegen paths.
    pub fn is_decimal(&self) -> bool {
        matches!(
            self,
            CType::Decimal32 | CType::Decimal64 | CType::Decimal128
        )
    }

    /// Decimal precision rank: 0 = not decimal, else 32 < 64 < 128.
    pub fn decimal_rank(&self) -> u8 {
        match self {
            CType::Decimal32 => 1,
            CType::Decimal64 => 2,
            CType::Decimal128 => 3,
            _ => 0,
        }
    }

    /// Whether this is `_Float16` (`__fp16`) or `__bf16` (16-bit floating point).
    pub fn is_half_float(&self) -> bool {
        matches!(self, CType::Float16 | CType::BFloat16)
    }

    /// Whether values of this type live in an integer carrier and take
    /// their arithmetic, comparisons, conversions and truthiness from
    /// CType-level lowering: the decimal types (libbid), _Float128 (libgcc),
    /// and _Float16 / __bf16 (hardware F16C/FCVT or inline IEEE-754 RNE).
    pub fn is_soft_float(&self) -> bool {
        self.is_decimal() || self.is_half_float() || matches!(self, CType::Float128)
    }

    /// Whether this is an arithmetic type (integer, real floating, or
    /// complex). _Float128, _Float16 and __bf16 are real floating types and so
    /// arithmetic, although `is_floating()` excludes them: that predicate
    /// gates direct F32/F64/F128 IR FP codegen.
    pub fn is_arithmetic(&self) -> bool {
        self.is_integer()
            || self.is_floating()
            || self.is_half_float()
            || matches!(self, CType::Float128)
            || self.is_complex()
    }

    /// SysV AMD64 by-value argument convention of a struct, union or vector
    /// (`None` for other types and incomplete records). See `sysv_classify`.
    pub fn sysv_passing(
        &self,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<super::sysv_classify::SysvPassing> {
        super::sysv_classify::classify(self, ctx)
    }

    /// The return-position counterpart of `sysv_passing`.
    pub fn sysv_return_passing(
        &self,
        ctx: &dyn StructLayoutProvider,
    ) -> Option<super::sysv_classify::SysvPassing> {
        super::sysv_classify::classify_return(self, ctx)
    }

    /// Whether this is a GCC vector extension type.
    /// Whether this is a GCC vector extension type.
    /// SysV AMD64 classification of a VECTOR type, as the pair
    /// `(eightbyte classes, occupies ONE xmm register)`.
    ///
    /// The psABI classifies the FIRST eightbyte of a vector as SSE and every
    /// further eightbyte as SSEUP -- "the upper half of the same vector
    /// register" -- which is why `<4 x i32>` occupies exactly ONE `%xmm`
    /// register while a `{double, double}` struct, whose two eightbytes are
    /// both plain SSE, takes two.  LCCC already models "one 16-byte XMM" with
    /// the carrier `_Float128` uses (`is_f128_sse` in the call ABI), so a
    /// 16-byte vector is described by the same flag.
    ///
    /// Returns `None` for anything that is not a vector, and for 32-byte
    /// vectors, whose SysV treatment (two SSEUP eightbytes in ONE `%ymm`) is a
    /// per-invocation ABI decision the caller has to make with `-mavx` in
    /// hand -- returning `None` keeps the existing by-value behaviour there
    /// instead of promising a convention the backend may not emit.
    pub fn sysv_vector_carrier(&self) -> Option<(Vec<EightbyteClass>, bool)> {
        if !self.is_vector() {
            return None;
        }
        // Clang lowers packed `_Bool` ext vectors through an integer carrier
        // on x86-64 (e.g. ext_vector_type(4) is an i8 argument/return), not an
        // SSE register. This classification is separate from vector_size.
        if self.is_bitpacked_bool_vector() {
            return match self.size() {
                1..=8 => Some((vec![EightbyteClass::Integer], false)),
                _ => None,
            };
        }
        // ONE-element floating-point vectors, per oracle, per spelling:
        //   - `vector_size`: None — GCC 16.2.0 measured V1SF/V1DF as MEMORY
        //     (stack argument, sret return), and the indirect convention
        //     must equal the direct one — `classify` answers
        //     SysvPassing::Memory for those types, so the sret path is
        //     taken.
        //   - `ext_vector_type` double: SSE — the pinned Clang 23.1.3
        //     returns it in the low half of %xmm0 (the gate asserts the
        //     -O1 chained-call shape end to end; the backend's packed-result
        //     spill fix keeps `dadd(did(x), dadd(y, y))` alive at -O1).
        //   - `ext_vector_type` float: falls through to the `1..=4` INTEGER
        //     arm below — the pinned Clang 23.1.3 ext_vector_abi_grid
        //     (exact expectations, all four link directions) returns it as
        //     INTEGER bits in %eax/%rax, like u32x1. The "scalar in xmm0"
        //     reading came from GCC 16 evidence, and GCC 16 does not accept
        //     the attribute at all — no oracle for this spelling.
        // 32-bit targets never reach this classifier (their ext vectors use
        // the GP-word path).
        if self.elem_is_fp_scalar() {
            if matches!(self, CType::Vector(_, _)) {
                return None;
            }
            return match self.size() {
                8 => Some((vec![EightbyteClass::Sse], false)),
                _ => None,
            };
        }
        match self.size() {
            // Smaller than an eightbyte: INTEGER class, carried in the low
            // bytes of ONE general-purpose register. Measured with both
            // oracles on x86-64: clang-19 `id_u8x1`/`id_u8x2`/`id_u16x2`/
            // `id_u32x1` and gcc-16 `vector_size(1,2,4)` shapes all arrive
            // in %edi and leave in %eax. The old rule classified these as
            // SSE and LCCC passed them in %xmm0, which no oracle does.
            1..=4 => Some((vec![EightbyteClass::Integer], false)),
            // 5..=8: A ONE-lane integer vector (u64x1/i64x1) is SysV-INTEGER
            // — it arrives in %rdi and returns in %rax, exactly like
            // uint64_t. GCC 12/16 and Clang agree (the psABI classes an
            // 8-byte aggregate whose single member is INTEGER as INTEGER;
            // "one lane" is the trivial one-member aggregate). Without this,
            // the backend routes a 64-bit integer one-lane through %xmm0 and
            // emits `movq %xmm0,%rdi; movq %rdi,%xmm0; ret` at every leaf:
            // three wasted instructions plus a false XMM/GPR dependency
            // (domain-crossing merge uop). Multi-lane integer vectors
            // (u32x2, u16x4, ...) stay SSE — they are NOT scalar-carried.
            5..=8
                if self
                    .vector_info()
                    .is_some_and(|(e, n)| n == 1 && e.is_integer()) =>
            {
                Some((vec![EightbyteClass::Integer], false))
            }
            // One SSE eightbyte: the low 64 bits of ONE xmm register.
            5..=8 => Some((vec![EightbyteClass::Sse], false)),
            // SSE + SSEUP: the full 128 bits of ONE xmm register. An
            // ext_vector_type(3) object is padded to 16 bytes and still uses a
            // single %xmm0 (clang-19 `id_u32x3`/`id_f32x3`), so 9..=16 is one
            // register, not the two-register struct form.
            9..=16 => Some((vec![EightbyteClass::Sse, EightbyteClass::Sse], true)),
            _ => None,
        }
    }

    pub fn is_vector(&self) -> bool {
        matches!(self, CType::Vector(_, _) | CType::ExtVector(_, _))
    }

    /// Whether this vector's sole element lane is a floating-point scalar
    /// (float/double), used by the single-lane SSE carrier rules above. Only
    /// meaningful when `self.is_vector()` is true; returns false for
    /// multi-lane vectors (their classification is handled by the
    /// per-eightbyte rules) and for integer vectors.
    pub fn elem_is_fp_scalar(&self) -> bool {
        match self {
            CType::ExtVector(elem, layout) => layout.lanes == 1 && elem.is_floating(),
            CType::Vector(elem, total_size) => {
                let es = elem.size();
                es > 0 && *total_size == es && elem.is_floating()
            }
            _ => false,
        }
    }

    /// Whether this is a bit-packed bool ext-vector. `vector_size` types are
    /// deliberately excluded and retain their existing byte-stride semantics.
    pub fn is_bitpacked_bool_vector(&self) -> bool {
        matches!(self, CType::ExtVector(_, layout) if layout.bitpacked_bool)
    }

    /// The 16-byte pieces LLVM widens or splits this ext vector into on
    /// x86 (its calling-convention register type), or `None` when the vector
    /// is a single scalar lane or is scalarized instead. Measured against
    /// Clang 23.1.3: `_Bool` lanes widen at 2, 4, 8 and 16 lanes; other lanes
    /// when the lane count is a power of two, or when rounding it up to one
    /// fills exactly 16 bytes (u8x12, u16x5..7, u32x3, f32x3).
    pub fn ext_vector_xmm_pieces(&self) -> Option<usize> {
        let CType::ExtVector(elem, layout) = self else {
            return None;
        };
        let n = layout.lanes;
        if layout.bitpacked_bool {
            return matches!(n, 2 | 4 | 8 | 16).then_some(1);
        }
        let e = elem.size();
        if n < 2 || matches!(**elem, CType::LongDouble) || !matches!(e, 1 | 2 | 4 | 8) {
            return None;
        }
        if n.is_power_of_two() {
            Some((n * e).div_ceil(16))
        } else {
            (n.next_power_of_two() * e == 16).then_some(1)
        }
    }

    /// Whether this type is a 16-byte SIMD vector on i386: the GNU
    /// `vector_size(16)` form with integer/FP lanes, or a Clang
    /// `ext_vector_type` occupying exactly one 16-byte XMM piece. This is
    /// the i386 psABI "16-byte class": an XMM carrier under SSE, and —
    /// independent of the baseline — a 16-byte-aligned slot in the
    /// variadic argument area and in `va_arg` (the GCC 4.6 va rule;
    /// measured on GCC 16.2 and Clang 23.1.3, which agree on both the
    /// caller padding and the va_arg realignment).
    pub fn is_i386_16b_simd_vector(&self) -> bool {
        if target_elf_machine() != 3 {
            return false;
        }
        match self {
            CType::Vector(elem, 16) => {
                elem.is_integer() || matches!(**elem, CType::Float | CType::Double)
            }
            CType::ExtVector(elem, layout) => {
                // One XMM can *hold* an 8-byte vector too, so
                // `ext_vector_xmm_pieces() == Some(1)` alone is not the
                // 16-byte class: require the type to actually occupy 16
                // bytes (u8x8/u16x4/u32x2 stay the ordinary aggregate path).
                self.size() == 16
                    && !layout.bitpacked_bool
                    && (elem.is_integer() || matches!(**elem, CType::Float | CType::Double))
                    && self.ext_vector_xmm_pieces() == Some(1)
            }
            _ => false,
        }
    }

    /// GNU 16-byte vectors share the single-XMM i386 ext-vector carrier only
    /// when this invocation has SSE. Explicit `-mno-sse` (or an equivalent
    /// general-register restriction) retains the ordinary aggregate
    /// stack/sret convention; SSE-only `-mno-sse2` keeps the XMM ABI. Keep
    /// the other vector_size widths and non-x86 targets separate.
    fn i386_gnu_xmm_vector(&self) -> bool {
        target_elf_machine() == 3
            && crate::common::x86_features::effective()
                .contains(crate::common::x86_features::X86Feature::Sse)
            && matches!(self, CType::Vector(elem, 16)
                if elem.is_integer() || matches!(**elem, CType::Float | CType::Double))
    }

    /// Clang 23.1.3's i386 convention for this ext vector as an argument
    /// or a GNU 16-byte vector (`None` for other types).
    /// See `I386ExtVectorCarrier`.
    pub fn i386_ext_vector_arg(&self) -> Option<I386ExtVectorCarrier> {
        if self.i386_gnu_xmm_vector() {
            return Some(I386ExtVectorCarrier::Xmm { pieces: 1 });
        }
        let CType::ExtVector(elem, layout) = self else {
            return None;
        };
        let n = layout.lanes;
        // The XMM carrier exists only on a baseline that actually has SSE:
        // Clang 23.1.3 `-m32 -mno-sse` passes a 16-byte ext vector as an
        // ordinary 4-granular by-value aggregate (measured: no XMM in the
        // callee at all). Returning `None` pre-SSE hands the argument to
        // the aggregate path in every consumer (decompose, classify,
        // prologue) — with the one deliberate exception, handled in the
        // caller lowering, that a VARIADIC 16-byte class is still
        // 16-byte aligned (GCC 4.6 va rule, `is_i386_16b_simd_vector`).
        let sse = crate::common::x86_features::effective()
            .contains(crate::common::x86_features::X86Feature::Sse);
        Some(if n == 1 {
            I386ExtVectorCarrier::Lanes
        } else if layout.bitpacked_bool && n > 64 {
            I386ExtVectorCarrier::Unsupported
        } else if (layout.bitpacked_bool && n == 64)
            || (!layout.bitpacked_bool && elem.is_integer() && n * elem.size() == 8)
        {
            I386ExtVectorCarrier::Image64
        } else if let Some(pieces) = self.ext_vector_xmm_pieces() {
            if sse {
                I386ExtVectorCarrier::Xmm { pieces }
            } else {
                return None;
            }
        } else {
            I386ExtVectorCarrier::Lanes
        })
    }

    /// Number of IR arguments an i386 by-value ext vector expands to: one
    /// per lane when scalarized, one per 16-byte piece when widened, and one
    /// otherwise (including every non-ext-vector type).
    pub fn i386_ext_vector_ir_args(&self) -> usize {
        match self.i386_ext_vector_arg() {
            Some(I386ExtVectorCarrier::Lanes) => self.vector_info().map_or(1, |(_, lanes)| lanes),
            Some(I386ExtVectorCarrier::Xmm { pieces }) => pieces,
            _ => 1,
        }
    }

    /// Clang 23.1.3's i386 convention for this ext vector as a return value
    /// or a GNU 16-byte vector (`None` for other types).
    /// See `I386ExtVectorCarrier`.
    pub fn i386_ext_vector_ret(&self) -> Option<I386ExtVectorCarrier> {
        if self.i386_gnu_xmm_vector() {
            return Some(I386ExtVectorCarrier::Xmm { pieces: 1 });
        }
        let CType::ExtVector(elem, layout) = self else {
            return None;
        };
        let n = layout.lanes;
        let int_lanes = layout.bitpacked_bool || elem.is_integer();
        // XMM return under SSE only — the same measured fork as
        // `i386_ext_vector_arg`: pre-SSE 16-byte ext vectors return
        // through the ordinary aggregate path (hidden pointer on i386;
        // GCC 16.2 and Clang 23.1.3 both do so, measured).
        let sse = crate::common::x86_features::effective()
            .contains(crate::common::x86_features::X86Feature::Sse);
        Some(if n == 1 {
            I386ExtVectorCarrier::Scalar
        } else if layout.bitpacked_bool && n > 64 {
            I386ExtVectorCarrier::Unsupported
        } else if let Some(pieces) = self.ext_vector_xmm_pieces() {
            if !sse {
                return None;
            }
            match pieces {
                1 => I386ExtVectorCarrier::Xmm { pieces },
                // %xmm0..%xmm3: no lccc carrier for a multi-register return.
                2..=4 => I386ExtVectorCarrier::Unsupported,
                _ => I386ExtVectorCarrier::Sret,
            }
        } else if int_lanes && n <= 3 && elem.size() <= 4 {
            I386ExtVectorCarrier::GpLanes
        } else if matches!(**elem, CType::LongDouble) && n == 2 {
            // %st(0) and %st(1).
            I386ExtVectorCarrier::Unsupported
        } else {
            I386ExtVectorCarrier::Sret
        })
    }

    /// The IR carrier of an i386 ext-vector register return: up to three
    /// lane words in EAX/EDX/ECX (I32, I64, or the I128 bundle tagged with
    /// three INTEGER classes), a one-lane floating vector as its x87 scalar,
    /// or one XMM piece (I128 tagged `[Sse, SseUp]`). `None` for sret and
    /// unsupported returns and types without a supported vector carrier.
    pub fn i686_ext_vector_ret_carrier(&self) -> Option<(IrType, Vec<EightbyteClass>)> {
        match self.i386_ext_vector_ret()? {
            I386ExtVectorCarrier::Xmm { .. } => Some((
                IrType::I128,
                vec![EightbyteClass::Sse, EightbyteClass::SseUp],
            )),
            I386ExtVectorCarrier::Scalar | I386ExtVectorCarrier::GpLanes => {
                if let Some(words) = self.i686_ext_vector_gp_word_count() {
                    return Some(match words {
                        1 => (IrType::I32, Vec::new()),
                        2 => (IrType::I64, Vec::new()),
                        _ => (IrType::I128, vec![EightbyteClass::Integer; 3]),
                    });
                }
                let (elem, _) = self.vector_info()?;
                matches!(elem, CType::Float | CType::Double | CType::LongDouble)
                    .then(|| (IrType::from_ctype(elem), Vec::new()))
            }
            _ => None,
        }
    }

    /// Whether Clang 23.1.3's x86-64 convention for this ext vector has no
    /// lccc carrier in the given position, so passing or returning it by
    /// value must be rejected rather than silently disagree. Arguments:
    /// `_Bool` vectors over 64 lanes (LLVM spreads them one lane per stack
    /// byte slot). Returns over 16 bytes that LLVM still keeps in registers:
    /// two to four %xmm pieces and three 8-byte lanes (%rax/%rdx/%rcx, or
    /// %xmm0/%xmm1/%st(0)). Two `long double` lanes (%st(0)/%st(1)) are
    /// implemented (`SysvPassing::X87Pair`). Every other larger return is
    /// sret, as lccc's MEMORY class does.
    pub fn x86_64_ext_vector_unsupported(&self, is_return: bool) -> bool {
        let CType::ExtVector(elem, layout) = self else {
            return false;
        };
        if layout.bitpacked_bool && layout.lanes > 64 {
            return !is_return;
        }
        if !is_return || self.size() <= 16 {
            return false;
        }
        // Under the AVX / AVX-512F ABI level Clang classifies a vector of up
        // to the native register width (`getNativeVectorSizeForAVXABI`:
        // 256 / 512 bits, a three-lane type rounded up to the next power of
        // two by `ASTContext::getTypeInfo`) as SSE + SSEUP, i.e. ONE %ymm /
        // %zmm, for returns as for named arguments -- the same carrier the
        // `vector_size` ABI uses (AUD-VABI-2), which lccc implements. The
        // piece-wise register returns below are the no-AVX convention only.
        if !layout.bitpacked_bool
            && self.size().next_power_of_two() <= target_vector_abi_bytes()
            && !matches!(**elem, CType::LongDouble)
        {
            return false;
        }
        self.ext_vector_xmm_pieces()
            .is_some_and(|p| (2..=4).contains(&p))
            || (layout.lanes == 3 && elem.size() == 8)
    }

    /// Number of 32-bit i386 GP return words used for a Clang integer
    /// ext-vector returned lane by lane (`GpLanes`, or a lone integer lane):
    /// lane i in the i-th of EAX, EDX and ECX, a 64-bit lane in EAX:EDX.
    /// Deliberately limited to ExtVector; `vector_size` is unchanged.
    pub fn i686_ext_vector_gp_word_count(&self) -> Option<usize> {
        let CType::ExtVector(elem, layout) = self else {
            return None;
        };
        if !matches!(
            self.i386_ext_vector_ret(),
            Some(I386ExtVectorCarrier::GpLanes | I386ExtVectorCarrier::Scalar)
        ) {
            return None;
        }
        if !elem.is_integer() {
            return None;
        }
        let elem_size = elem.size();
        if elem_size == 0 {
            return None;
        }
        let words_per_lane = elem_size.checked_add(3)?.checked_div(4)?;
        let words = layout.lanes.checked_mul(words_per_lane)?;
        (1..=3).contains(&words).then_some(words)
    }

    /// The lanes the Clang ext-vector element selection `self.<name>` names
    /// (see [`ext_vector_selector_lanes`]). `None` when `self` is not an ext
    /// vector; an error for every selector on a `_Bool` ext vector, which
    /// Clang 23.1.3 rejects quoting the whole name (`b.x`, `b.lo`, `b.s1`).
    pub fn ext_vector_selection(
        &self,
        name: &str,
    ) -> Option<Result<Vec<usize>, ExtVectorSelectorError>> {
        let CType::ExtVector(_, layout) = self else {
            return None;
        };
        if layout.bitpacked_bool {
            return Some(Err(ExtVectorSelectorError::IllegalName(0..name.len())));
        }
        Some(ext_vector_selector_lanes(name, layout.lanes))
    }

    /// The lanes of a valid ext-vector element selection `self.<name>`;
    /// `None` for a non-ext-vector or an invalid selector (sema diagnoses).
    pub fn ext_vector_swizzle(&self, name: &str) -> Option<Vec<usize>> {
        self.ext_vector_selection(name)?.ok()
    }

    /// The type of the component access `self.name`: the element type for a
    /// single component, otherwise an ext vector of the element with as
    /// many lanes as components named (`float4.xy` is `float2`).
    pub fn ext_vector_swizzle_ctype(&self, name: &str) -> Option<CType> {
        let lanes = self.ext_vector_swizzle(name)?;
        let CType::ExtVector(elem, _) = self else {
            return None;
        };
        Self::ext_vector_selection_ctype(elem, lanes.len())
    }

    /// The type of an ext-vector selection of `count` lanes of `elem`: the
    /// element itself for one lane, else an ext vector of `count` lanes.
    pub fn ext_vector_selection_ctype(elem: &CType, count: usize) -> Option<CType> {
        if count == 1 {
            return Some(elem.clone());
        }
        let layout = ExtVectorLayout::new(elem, count)?;
        Some(CType::ExtVector(Box::new(elem.clone()), layout))
    }

    /// An ext vector of `long double` lanes: lanes are 10 bytes apart (the
    /// x87 80-bit image, `X87_LANE_STRIDE`), narrower than the 16-byte IR
    /// `F128` store, so a lane store must write exactly 10 bytes.
    pub fn is_x87_lane_vector(&self) -> bool {
        matches!(self, CType::ExtVector(elem, _) if matches!(**elem, CType::LongDouble))
    }

    /// Storage stride for one logical vector lane, or None for a packed bool
    /// ext-vector whose lane addresses are bit positions rather than bytes.
    pub fn vector_element_storage_size(&self) -> Option<usize> {
        match self {
            CType::Vector(elem, _) => Some(elem.size()),
            CType::ExtVector(_, layout) if !layout.bitpacked_bool => {
                Some(layout.element_storage_size)
            }
            CType::ExtVector(_, _) => None,
            _ => None,
        }
    }

    /// The type of a vector comparison whose operand has this vector type:
    /// signed integer lanes of the same width and count (GCC and Clang:
    /// `int` lanes for `float`, 64-bit lanes for `double`). An ext-vector
    /// keeps its layout, which depends only on the lane count and width; a
    /// bit-packed `_Bool` ext-vector compares to its own type. `None` for
    /// non-vectors and lane widths without a signed integer type.
    pub fn vector_comparison_result(&self) -> Option<CType> {
        let signed_lane = |elem: &CType| -> Option<CType> {
            Some(match elem.size() {
                // GCC and Clang: `__vector signed char`
                1 => CType::signed_char(),
                2 => CType::Short,
                4 => CType::Int,
                8 => CType::LongLong,
                16 => CType::Int128,
                _ => return None,
            })
        };
        match self {
            CType::Vector(elem, total) => Some(CType::Vector(Box::new(signed_lane(elem)?), *total)),
            CType::ExtVector(_, layout) if layout.bitpacked_bool => Some(self.clone()),
            CType::ExtVector(elem, layout) => {
                Some(CType::ExtVector(Box::new(signed_lane(elem)?), *layout))
            }
            _ => None,
        }
    }

    /// `vector_info` for a type the caller has already classified as a
    /// vector (an ext-vector ABI carrier was selected for it).  A non-vector
    /// here is a lowering invariant violation, reported as an internal error
    /// at this one site rather than at every carrier.
    pub fn vector_lanes(&self) -> (&CType, usize) {
        self.vector_info()
            .expect("ExtVector must carry its logical lane count")
    }

    /// For a vector type, returns (element_type, logical lane count).
    /// Returns None for non-vector types. Ext-vector lane count is never
    /// inferred from padded object storage.
    /// The vector type of the same kind and element type with `lanes` lanes
    /// (a GNU `vector_size` vector or a Clang ext vector): the result type of
    /// `__builtin_shufflevector`, which has one lane per index.
    pub fn with_lane_count(&self, lanes: usize) -> Option<CType> {
        match self {
            CType::Vector(elem, _) => {
                let bytes = elem.size().checked_mul(lanes)?;
                (bytes > 0).then(|| CType::Vector(elem.clone(), bytes))
            }
            CType::ExtVector(elem, _) => ExtVectorLayout::new(elem, lanes)
                .map(|layout| CType::ExtVector(elem.clone(), layout)),
            _ => None,
        }
    }

    /// `__builtin_shufflevector`'s result for `lanes` indices, or `None` for
    /// an invalid count. GNU vectors need a power of two (GCC 16.2; a
    /// `vector_size` type has no other shape). Clang ext vectors take any
    /// count, as in Clang: three indices on a `float4` give a `float3`.
    pub fn shufflevector_result(&self, lanes: usize) -> Option<CType> {
        match self {
            CType::Vector(..) if !lanes.is_power_of_two() => None,
            _ if lanes == 0 => None,
            _ => self.with_lane_count(lanes),
        }
    }

    /// Result type of a call to GCC's vector shuffle builtins, given the type
    /// of the first argument and the argument count: `__builtin_shuffle`
    /// returns the first vector's type, `__builtin_shufflevector` that element
    /// type with one lane per index. An invalid index count keeps the first
    /// vector's type, so the lowering's diagnostic for it
    /// is the one reported. Sema and the lowerer (which types `typeof`
    /// operands) both use this, so the two cannot disagree.
    pub fn shuffle_builtin_result(name: &str, first: &CType, args: usize) -> Option<CType> {
        if !first.is_vector() {
            return None;
        }
        match name {
            "__builtin_shuffle" => Some(first.clone()),
            "__builtin_shufflevector" => Some(
                args.checked_sub(2)
                    .and_then(|lanes| first.shufflevector_result(lanes))
                    .unwrap_or_else(|| first.clone()),
            ),
            _ => None,
        }
    }

    pub fn vector_info(&self) -> Option<(&CType, usize)> {
        match self {
            CType::Vector(elem, total_size) => {
                let elem_size = elem.size();
                if elem_size > 0 {
                    Some((elem.as_ref(), total_size / elem_size))
                } else {
                    None
                }
            }
            CType::ExtVector(elem, layout) => Some((elem.as_ref(), layout.lanes)),
            _ => None,
        }
    }

    /// Apply C integer promotion rules (C11 6.3.1.1):
    /// Types smaller than int are promoted to int (or unsigned int if int cannot
    /// represent all values, but for our targets int is 32-bit so Bool/Char/UChar/
    /// Short/UShort all fit in int).
    pub fn integer_promoted(&self) -> CType {
        match self {
            CType::Bool
            | CType::Char
            | CType::SChar
            | CType::UChar
            | CType::PlainUChar
            | CType::Short
            | CType::UShort => CType::Int,
            // An enum promotes as its compatible type does (C11 6.3.1.1p2): a
            // packed one to `int`, an `unsigned int` one stays unsigned.
            CType::Enum(e) => e.compatible_ctype().integer_promoted(),
            other => other.clone(),
        }
    }

    /// Get the component type for a complex type (e.g., ComplexFloat -> Float).
    pub fn complex_component_type(&self) -> CType {
        match self {
            CType::ComplexFloat => CType::Float,
            CType::ComplexDouble => CType::Double,
            CType::ComplexLongDouble => CType::LongDouble,
            _ => self.clone(), // not complex, return self
        }
    }

    /// Whether this is an unsigned integer type.
    /// Used by usual arithmetic conversions (C11 6.3.1.8).
    pub fn is_unsigned(&self) -> bool {
        if let CType::Enum(e) = self {
            return e.compatible_ctype().is_unsigned();
        }
        matches!(
            self,
            CType::Bool
                | CType::UChar
                | CType::PlainUChar
                | CType::UShort
                | CType::UInt
                | CType::ULong
                | CType::ULongLong
                | CType::UInt128
        )
    }

    /// Integer conversion rank for C types (C11 6.3.1.1).
    /// Higher rank = larger type. Used by usual arithmetic conversions.
    pub fn integer_rank(&self) -> u32 {
        match self {
            CType::Bool => 0,
            CType::Char | CType::SChar | CType::UChar | CType::PlainUChar => 1,
            CType::Short | CType::UShort => 2,
            CType::Int | CType::UInt => 3,
            // An enum ranks as its compatible type (C11 6.3.1.1p1).
            CType::Enum(e) => e.compatible_ctype().integer_rank(),
            CType::Long | CType::ULong => 4,
            CType::LongLong | CType::ULongLong => 5,
            CType::Int128 | CType::UInt128 => 6,
            _ => 3, // fallback to int rank for non-integer types
        }
    }

    /// Apply C "usual arithmetic conversions" (C11 6.3.1.8) to determine
    /// the common type of two operands in a binary arithmetic expression.
    ///
    /// When one operand is complex and the other is real, the corresponding
    /// real type of the complex is compared with the other operand's type to
    /// determine the wider type, and the result is the complex version.
    pub fn usual_arithmetic_conversion(lhs: &CType, rhs: &CType) -> CType {
        // Handle vector types per GCC vector extension semantics:
        // When one operand is a vector and the other is a scalar, the result
        // is the vector type. The scalar is implicitly splatted to all elements.
        if lhs.is_vector() {
            return lhs.clone();
        }
        if rhs.is_vector() {
            return rhs.clone();
        }

        // First apply integer promotions
        let l = lhs.integer_promoted();
        let r = rhs.integer_promoted();

        // Handle complex types per C11 6.3.1.8
        let l_complex = l.is_complex();
        let r_complex = r.is_complex();
        if l_complex || r_complex {
            let l_real_rank = match &l {
                CType::ComplexLongDouble | CType::LongDouble => 3,
                CType::ComplexDouble | CType::Double => 2,
                CType::ComplexFloat | CType::Float => 1,
                _ => 0,
            };
            let r_real_rank = match &r {
                CType::ComplexLongDouble | CType::LongDouble => 3,
                CType::ComplexDouble | CType::Double => 2,
                CType::ComplexFloat | CType::Float => 1,
                _ => 0,
            };
            let max_rank = l_real_rank.max(r_real_rank);
            return match max_rank {
                3 => CType::ComplexLongDouble,
                2 => CType::ComplexDouble,
                1 => CType::ComplexFloat,
                _ => {
                    if l_complex {
                        l.clone()
                    } else {
                        r.clone()
                    }
                }
            };
        }

        // Decimal floating point (C23/TS 18661-2): decimal types form their
        // own hierarchy and absorb integers, but never mix with binary
        // floats in the usual arithmetic conversions (GCC errors on that
        // combination; we fall through to the binary-float hierarchy, which
        // requires an explicit __bid_* conversion the lowering provides).
        let ld = l.decimal_rank();
        let rd = r.decimal_rank();
        if ld > 0 || rd > 0 {
            if ld > 0 && rd > 0 {
                return if ld >= rd { l.clone() } else { r.clone() };
            }
            if ld > 0 && !r.is_floating() {
                return l.clone(); // integer operand converts to decimal
            }
            if rd > 0 && !l.is_floating() {
                return r.clone();
            }
            // one decimal + one binary float: binary float wins (see above)
        }

        // _Float128 (IEEE binary128) holds every value of every standard
        // floating type on every lccc target (x87 extended has the same
        // exponent range and 64 < 113 significand bits; elsewhere long
        // double is binary128 itself or double), so it is the common type of
        // any mix with a real arithmetic type, integers included (C23
        // 6.3.1.8; GCC: the interchange type wins over a long double of the
        // same format). Without this arm a float or double operand won and
        // an integer one was ranked like `int`.
        if matches!(&l, CType::Float128) || matches!(&r, CType::Float128) {
            return CType::Float128;
        }

        // Non-complex: standard type hierarchy
        if matches!(&l, CType::LongDouble) || matches!(&r, CType::LongDouble) {
            return CType::LongDouble;
        }
        if matches!(&l, CType::Double) || matches!(&r, CType::Double) {
            return CType::Double;
        }
        if matches!(&l, CType::Float) || matches!(&r, CType::Float) {
            return CType::Float;
        }
        if matches!(&l, CType::Float16) || matches!(&r, CType::Float16) {
            return CType::Float16;
        }
        if matches!(&l, CType::BFloat16) || matches!(&r, CType::BFloat16) {
            return CType::BFloat16;
        }

        // Both integer types after promotion: apply conversion rank rules
        let l_rank = l.integer_rank();
        let r_rank = r.integer_rank();
        let l_unsigned = l.is_unsigned();
        let r_unsigned = r.is_unsigned();

        if l_unsigned == r_unsigned {
            if l_rank >= r_rank { l } else { r }
        } else if l_unsigned && l_rank >= r_rank {
            l
        } else if r_unsigned && r_rank >= l_rank {
            r
        } else {
            // The signed type has higher rank (C11 6.3.1.8p1). Two sub-cases:
            //  - If the signed type can represent ALL values of the unsigned
            //    type (strictly wider, e.g. long long vs unsigned int on
            //    LP64), the result is the signed type.
            //  - Otherwise (same width, e.g. long long vs unsigned long on
            //    LP64, or long vs unsigned int on ILP32), the result is the
            //    UNSIGNED type corresponding to the signed type.
            let (signed_ty, unsigned_ty) = if l_unsigned { (r, l) } else { (l, r) };
            if signed_ty.size() > unsigned_ty.size() {
                signed_ty
            } else {
                signed_ty.to_unsigned_version()
            }
        }
    }

    /// The unsigned type corresponding to a signed integer type (C11 6.3.1.8).
    fn to_unsigned_version(self) -> CType {
        match self {
            CType::Char | CType::SChar => CType::UChar,
            CType::Short => CType::UShort,
            CType::Int => CType::UInt,
            CType::Long => CType::ULong,
            CType::LongLong => CType::ULongLong,
            CType::Int128 => CType::UInt128,
            other => other,
        }
    }

    /// Compute the composite type for a conditional expression per C11 6.5.15.
    ///
    /// Rules:
    /// - Both arithmetic: usual arithmetic conversions
    /// - Both void: void
    /// - Both pointers with one being void*:
    ///   - If the void* branch is a null pointer constant, result is the other pointer type
    ///   - Otherwise, result is void* (C11 6.5.15p6)
    /// - Both struct/union: return the then-branch type (must be compatible)
    /// - Otherwise: return the then-branch type as fallback
    ///
    /// The `then_is_npc`/`else_is_npc` flags indicate whether each branch is a
    /// null pointer constant (per C11 6.3.2.3p3). This matters for the kernel's
    /// `__is_constexpr` macro which relies on `sizeof` of a conditional where
    /// one branch is `(void*)runtime_zero` (NOT an NPC → type is void*, sizeof 1)
    /// vs `(void*)constant_zero` (IS an NPC → type is int*, sizeof 4).
    ///
    /// The rules operate on the arms' VALUES, so both arms undergo lvalue
    /// conversion first (C11 6.3.2.1p3-p4): `1 ? arr : arr` is an `int *`.
    /// Decaying here, not in each caller, keeps sema and lowering agreeing
    /// (sema once sized that conditional as the array, 16 bytes, not 8).
    /// Whether a `?:` arm's null-pointer-constant status can change
    /// [`Self::conditional_composite_type`]: only for a `void *` arm against
    /// a pointer to a non-void type, where `(void *)0` takes the other arm's
    /// type and any other `void *` makes the result `void *` (C11 6.5.15p6).
    /// An integer arm against a pointer yields the pointer type either way.
    pub fn conditional_arm_npc_matters(arm: Option<&CType>, other: Option<&CType>) -> bool {
        let pointee_is_void = |ct: &CType| match ct.decayed_value_ctype() {
            CType::Pointer(pointee, _) => Some(matches!(*pointee, CType::Void)),
            _ => None,
        };
        arm.and_then(pointee_is_void) == Some(true)
            && other.and_then(pointee_is_void) == Some(false)
    }

    /// Whether two pointee types are certainly not compatible (C11 6.2.7),
    /// judged only where `CType` carries enough to be sure: distinct basic
    /// arithmetic types, distinct structure/union tags, a record against a
    /// scalar, recursing through pointer levels. Enumerations (compatible
    /// with their underlying integer type), arrays, functions and vectors
    /// (composite-type rules) answer `false`.
    fn pointees_definitely_incompatible(a: &CType, b: &CType) -> bool {
        let basic = |ct: &CType| {
            matches!(
                ct,
                CType::Bool
                    | CType::Char
                    | CType::SChar
                    | CType::UChar
                    | CType::PlainUChar
                    | CType::Short
                    | CType::UShort
                    | CType::Int
                    | CType::UInt
                    | CType::Long
                    | CType::ULong
                    | CType::LongLong
                    | CType::ULongLong
                    | CType::Int128
                    | CType::UInt128
                    | CType::Float
                    | CType::Double
                    | CType::LongDouble
                    | CType::Float16
                    | CType::BFloat16
                    | CType::Float128
                    | CType::Decimal32
                    | CType::Decimal64
                    | CType::Decimal128
                    | CType::ComplexFloat
                    | CType::ComplexDouble
                    | CType::ComplexLongDouble
            )
        };
        let record = |ct: &CType| matches!(ct, CType::Struct(_) | CType::Union(_));
        match (a, b) {
            (CType::Pointer(pa, _), CType::Pointer(pb, _)) => {
                Self::pointees_definitely_incompatible(pa, pb)
            }
            _ if basic(a) && basic(b) => a != b,
            _ if record(a) && record(b) => a != b,
            _ => (basic(a) && record(b)) || (record(a) && basic(b)),
        }
    }

    /// Whether a `_Generic` association of type `assoc` matches a controlling
    /// operand of type `self`: compatible types (C23 6.5.1.1p2, 6.2.7) on
    /// lccc's qualifier-free `CType`, whose constness the callers compare
    /// through `GenericAssociation::is_const`. The operand type is taken as
    /// given -- callers apply lvalue conversion to an expression operand and
    /// none to a type operand -- and nothing decays below the top level:
    /// `int (*)[3]` is not compatible with `int **`, nor with `int (*)[4]`.
    /// An enumeration is compatible with its compatible integer type (which
    /// is `unsigned int` when no enumerator is negative), not with `int`.
    pub fn generic_association_matches(&self, assoc: &CType) -> bool {
        match (self, assoc) {
            (CType::Enum(_), CType::Enum(_)) => self == assoc,
            (CType::Enum(e), other) | (other, CType::Enum(e)) => e.compatible_ctype() == *other,
            (CType::Pointer(a, aq), CType::Pointer(b, bq)) => {
                aq.space == bq.space
                    && !aq.has_known_cvr_conflict(*bq)
                    && a.generic_association_matches(b)
            }
            (CType::Array(a, la), CType::Array(b, lb)) => {
                (la.is_none() || lb.is_none() || la == lb) && a.generic_association_matches(b)
            }
            // Parameter names are not part of the type. A prototype and an
            // unprototyped type use the same default-promotion compatibility
            // rule as declarations (C 6.7.6.3p15).
            (CType::Function(a), CType::Function(b)) => {
                let params_compatible = match (a.has_prototype, b.has_prototype) {
                    (true, true) => {
                        a.variadic == b.variadic
                            && a.params.len() == b.params.len()
                            && a.params
                                .iter()
                                .zip(&b.params)
                                .all(|((pa, _), (pb, _))| pa.generic_association_matches(pb))
                    }
                    (false, false) => !a.variadic && !b.variadic,
                    (true, false) => {
                        !a.variadic
                            && a.params.iter().all(|(param, _)| {
                                param
                                    .generic_association_matches(&default_argument_promotion(param))
                            })
                    }
                    (false, true) => {
                        !b.variadic
                            && b.params.iter().all(|(param, _)| {
                                param
                                    .generic_association_matches(&default_argument_promotion(param))
                            })
                    }
                };
                params_compatible && a.return_type.generic_association_matches(&b.return_type)
            }
            _ => self == assoc,
        }
    }

    /// This pointer type with its pointee's qualifiers replaced (see
    /// [`PtrQuals`]); any other type unchanged.
    #[must_use]
    pub fn with_pointee_cvr(self, cvr: Option<u8>) -> CType {
        match self {
            CType::Pointer(inner, quals) => CType::Pointer(inner, quals.with_cvr(cvr)),
            other => other,
        }
    }

    pub fn conditional_composite_type(
        then_ct: Option<CType>,
        else_ct: Option<CType>,
        then_is_npc: bool,
        else_is_npc: bool,
    ) -> Option<CType> {
        let then_ct = then_ct.map(|ct| ct.decayed_value_ctype());
        let else_ct = else_ct.map(|ct| ct.decayed_value_ctype());
        match (then_ct, else_ct) {
            (Some(t), Some(e)) => {
                // Both void
                if matches!(&t, CType::Void) && matches!(&e, CType::Void) {
                    return Some(CType::Void);
                }
                // C23 6.5.15p6: `nullptr_t` against a pointer is the pointer
                // type (both oracles: `c ? nullptr : (int *)p` is `int *`).
                // Two `nullptr_t` arms fall through to the shared type below.
                match (&t, &e) {
                    (CType::NullPtr, CType::Pointer(..)) => return Some(e),
                    (CType::Pointer(..), CType::NullPtr) => return Some(t),
                    _ => {}
                }
                // Both pointers: C11 6.5.15p6 rules
                if let (CType::Pointer(inner_t, t_quals), CType::Pointer(inner_e, e_quals)) =
                    (&t, &e)
                {
                    let t_is_void = matches!(inner_t.as_ref(), CType::Void);
                    let e_is_void = matches!(inner_e.as_ref(), CType::Void);
                    // A null pointer constant arm yields the other arm's type;
                    // otherwise the result points to a type with ALL the
                    // qualifiers of both pointees (recorded only if both are).
                    if then_is_npc && t_is_void {
                        return Some(e);
                    }
                    if else_is_npc && e_is_void {
                        return Some(t);
                    }
                    let joined = match (t_quals.cvr(), e_quals.cvr()) {
                        (Some(a), Some(b)) => Some(a | b),
                        _ => None,
                    };
                    if t_is_void && !e_is_void {
                        // then=void*, else=T*: void*
                        return Some(t.with_pointee_cvr(joined));
                    }
                    if e_is_void && !t_is_void {
                        // then=T*, else=void*: void*
                        return Some(e.with_pointee_cvr(joined));
                    }
                    // Preserve the composite type contributed by either
                    // compatible pointer operand. In particular, when one
                    // function pointer has a prototype and the other is
                    // unprototyped, the composite function type has that
                    // prototype; choosing the then-arm alone can incorrectly
                    // allow extra call arguments.
                    // 6.5.15 joins the immediate pointee qualification; it
                    // requires compatibility BELOW that qualification. Compare
                    // the inner types directly, avoiding two cloned stripped
                    // pointer trees just to ignore their immediate CVR.
                    if t_quals.space == e_quals.space {
                        if let Some(composite) = composite_compatible_type(inner_t, inner_e) {
                            return Some(CType::Pointer(
                                Box::new(composite),
                                t_quals.with_cvr(joined),
                            ));
                        }
                    }
                    // Pointers to incompatible types: GCC diagnoses the
                    // mismatch (an error since GCC 14, a warning under
                    // -fpermissive) and types the result `void *`; lccc
                    // accepts the mismatch, so the type must still be
                    // GCC's: `(c ? (int *)a : (char *)b) + 1` advances one
                    // byte, not four.
                    if Self::pointees_definitely_incompatible(inner_t, inner_e) {
                        let space = match &t {
                            CType::Pointer(_, quals) => PtrQuals::of(quals.space),
                            _ => PtrQuals::GENERIC,
                        };
                        return Some(CType::Pointer(Box::new(CType::Void), space));
                    }
                    return Some(t.with_pointee_cvr(None));
                }
                // One pointer, one integer: the pointer type, whether or not
                // the integer is a null pointer constant (GCC 16.2: `c ? 1 :
                // p` is a `char *` with a pointer/integer mismatch
                // diagnostic). Taking the integer arm's type truncated the
                // pointer arm to 32 bits.
                if let CType::Pointer(..) = &t {
                    if e.is_integer() {
                        return Some(t);
                    }
                }
                if let CType::Pointer(..) = &e {
                    if t.is_integer() {
                        return Some(e);
                    }
                }
                // Both arithmetic types: apply usual arithmetic conversions.
                // Decimals are deliberately excluded from is_arithmetic(),
                // but a decimal arm still selects the UAC common type (the
                // decimal hierarchy absorbing integers; binary float wins
                // over decimal per our UAC rule). Without this the fallback
                // below reports the then-arm type, and downstream implicit
                // conversions re-convert an already-decimal value (silent
                // miscompile: __bid_float* applied to BID carriers).
                if (t.is_arithmetic() && e.is_arithmetic()) || t.is_decimal() || e.is_decimal() {
                    return Some(CType::usual_arithmetic_conversion(&t, &e));
                }
                // Fallback: then-branch type (structs, unions, etc.)
                Some(t)
            }
            (Some(t), None) => Some(t),
            (None, Some(e)) => Some(e),
            (None, None) => None,
        }
    }

    /// The *value* conversion of an expression: array-to-pointer and
    /// function-to-pointer decay (C11 6.3.2.1p3 and p4).
    ///
    /// This is the conversion that applies to an expression **value**, so it is
    /// the single source of truth for every pass that needs the type of a value
    /// rather than the type of an object. `sizeof`'s array exception
    /// (C11 6.5.3.4p4, "the operand is not converted") covers only the operand
    /// *as written*; the results of the comma operator (C11 6.5.17p3) and of
    /// the conditional operator (C11 6.5.15) are values, so an array or
    /// function operand of either has already decayed and `sizeof` of the
    /// operator expression is the pointer size:
    ///
    /// ```c
    /// int a[4];
    /// sizeof((0, a));    // 8,  not 16: the operand is `(0, a)`, an int *
    /// sizeof(1 ? a : a); // 8,  not 16
    /// sizeof(a);         // 16: the exception applies to the operand itself
    /// ```
    ///
    /// Every other type is returned unchanged. Qualifiers are deliberately not
    /// stripped: callers that also need the lvalue-conversion qualifier drop
    /// (C11 6.3.2.1p2) must do it themselves, because a decayed `T *` value
    /// still carries the pointee qualifiers of `T`.
    pub fn decayed_value_ctype(&self) -> CType {
        match self {
            CType::Array(elem, _) => CType::Pointer(elem.clone(), PtrQuals::GENERIC),
            CType::Function(ft) => {
                CType::Pointer(Box::new(CType::Function(ft.clone())), PtrQuals::GENERIC)
            }
            other => other.clone(),
        }
    }

    /// Whether this is a pointer type (including arrays which decay to pointers).
    pub fn is_pointer_like(&self) -> bool {
        matches!(self, CType::Pointer(_, _) | CType::Array(_, _))
    }

    /// Whether this is a function pointer type: Pointer(Function(_)).
    pub fn is_function_pointer(&self) -> bool {
        matches!(self, CType::Pointer(inner, _) if matches!(inner.as_ref(), CType::Function(_)))
    }

    /// Extract the FunctionType from a function pointer or function type.
    /// Handles up to two levels of pointer indirection (e.g., Pointer(Pointer(Function)))
    /// to support pointer-to-function-pointer types used in indirect call expressions.
    /// Returns None if this is not a function or function pointer type.
    pub fn get_function_type(&self) -> Option<&FunctionType> {
        match self {
            CType::Function(ft) => Some(ft),
            CType::Pointer(inner, _) => match inner.as_ref() {
                CType::Function(ft) => Some(ft),
                CType::Pointer(inner2, _) => match inner2.as_ref() {
                    CType::Function(ft) => Some(ft),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// Extract the return type from a function pointer CType.
    ///
    /// Handles these CType shapes:
    ///   1. Pointer(Function(ft))           -> ft.return_type
    ///   2. Pointer(Pointer(Function(ft)))   -> ft.return_type (ptr-to-func-ptr)
    ///   3. Function(ft)                     -> ft.return_type (bare function type)
    ///   4. Pointer(X) where X is not Function -> X (typedef lost Function node)
    ///
    /// The `strict` flag controls behavior for case 4 and non-matching cases:
    ///   - strict=true: returns None (used when only real function pointers are wanted)
    ///   - strict=false: returns Some(X) (used when typedef fallback is acceptable)
    pub fn func_ptr_return_type(&self, strict: bool) -> Option<CType> {
        match self {
            CType::Pointer(inner, _) => match inner.as_ref() {
                CType::Function(ft) => Some(ft.return_type.clone()),
                CType::Pointer(inner2, _) => match inner2.as_ref() {
                    CType::Function(ft) => Some(ft.return_type.clone()),
                    _ => {
                        if strict {
                            None
                        } else {
                            Some(inner.as_ref().clone())
                        }
                    }
                },
                other => {
                    if strict {
                        None
                    } else {
                        Some(other.clone())
                    }
                }
            },
            CType::Function(ft) => Some(ft.return_type.clone()),
            _ => None,
        }
    }

    /// Whether this is a struct or union type.
    pub fn is_struct_or_union(&self) -> bool {
        matches!(self, CType::Struct(_) | CType::Union(_))
    }
}

/// IR-level types (simpler than C types).
/// Signed and unsigned variants are tracked separately so that the backend
/// can choose sign-extension vs zero-extension appropriately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IrType {
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
    /// _Decimal32 BID bit container (C23 decimal floating point).
    /// Classified like F32 for call/return ABI (SSE on x86-64) but never
    /// reaches binary FP arithmetic: ops lower to __bid_* helper calls at
    /// the CType level.
    D32,
    /// _Decimal64 BID bit container; classified like F64 for ABI.
    D64,
    /// 128-bit floating point (long double on AArch64/RISC-V, 80-bit extended on x86-64).
    /// Computation is done in F64 precision; this type exists to ensure correct
    /// ABI handling (16-byte storage, proper variadic argument passing, va_arg).
    F128,
    Ptr,
    #[default]
    Void,
}

impl IrType {
    /// Whether `Cast(self -> to)` preserves the integer VALUE across the whole
    /// source domain.
    ///
    /// This is the rule every transform that replaces an expression with a
    /// *mathematically linear* one has to satisfy: widening an unsigned source,
    /// or a signed source into a signed target, and nothing else. A same-width
    /// signedness reinterpretation fails it, because a LATER widening extends
    /// the result differently — `(int32_t)u` with `u == UINT32_MAX` is `-1`, and
    /// `sext(-1)` is not `zext(UINT32_MAX)`.
    ///
    /// `src/backend/generation.rs`'s SIB-index peel stated this rule inline (and
    /// documented the `I32 -> U32 -> I64` miscompile it prevents);
    /// `src/passes/iv_strength_reduce.rs` stated a weaker
    /// `to_ty.size() >= from_ty.size()` version of it and consequently turned
    /// `p[(int32_t)i]` with `i` crossing `UINT32_MAX` into a pointer recurrence
    /// that walked off the object (IVSR-DOMAIN-1, SIGSEGV at -O2 on two shipped
    /// benchmark shapes). Both now call this one function, so the rule cannot
    /// drift between them again; `cast_predicate_tests` below walks all 64
    /// ordered integer-type pairs against each type's actual domain extrema.
    ///
    /// Note this is NOT the right predicate for an offset that is consumed in
    /// the target's pointer ring — see [`IrType::cast_preserves_offset_value`].
    pub fn cast_preserves_integer_value(self, to: IrType) -> bool {
        self.is_integer()
            && to.is_integer()
            && (self == to
                || (to.size() > self.size() && (self.is_unsigned() || !to.is_unsigned())))
    }

    /// Whether `Cast(self -> to)` preserves the OFFSET VALUE that reaches a
    /// pointer computation.
    ///
    /// Deliberately not the same question as
    /// [`IrType::cast_preserves_integer_value`]. Address arithmetic lives in the
    /// target's pointer ring, so a same-width signedness reinterpretation AT OR
    /// ABOVE the pointer width cannot change the value the `GetElementPtr`
    /// consumes: on LP64, `(ptrdiff_t)(size_t)x` and `(size_t)x` are the same
    /// 64-bit pattern, and a recurrence `base + k*step*stride` is formed in that
    /// same ring. Narrower reinterpretations stay rejected, because there the
    /// pattern is re-extended afterwards and the two spellings diverge.
    ///
    /// Using the value-only predicate here is measurably wrong in the other
    /// direction: five programs in `tests/benchmark/programs` spell their 64-bit
    /// index `U64 -> I64`, and rejecting that cast cost `histogram` +5,
    /// `linux_rbtree` +12 instructions/+7 stack refs, `linux_find_bit` +1,
    /// `i686_alu_chains` +2 and `zlib_ng_adler32_combine` +1, partly offset by
    /// `sqlite_varint` −6 — over the 55-program default corpus that is
    /// +21 −6 = **+15 net** (8139 → 8154 instructions). Evidence:
    /// `engineering/evidence/2026-10-06-ivsr-domain/corpus-ab-value-only-predicate.json`.
    ///
    /// `src/passes/loop_carried_forward.rs` peels same-width integer/pointer
    /// casts under the same reasoning, expressed structurally: that analysis
    /// only ever follows pointer-width integers (`is_ptr_width_int`), so its
    /// same-width peel is exactly this predicate's pointer-ring arm.
    pub fn cast_preserves_offset_value(self, to: IrType) -> bool {
        self.cast_preserves_integer_value(to)
            || (self.is_integer()
                && to.is_integer()
                && self.size() == to.size()
                && (self.size() as usize) >= target_ptr_size())
    }
    /// Memory extent of one scalar `Load`/`Store` of this type, as
    /// `(definitely, possibly)` accessed bytes from the address, for the
    /// middle end's memory disambiguation (DSE, store-to-load forwarding,
    /// aggregate copy forwarding).  An overwrite may only be proven with the
    /// `definitely` extent of the killer and the `possibly` extent of the
    /// victim; every conflict check uses `possibly`.
    ///
    /// The two differ only for the x87 long double (F128 on x86 targets):
    /// `fldt`/`fstpt` touch its 10 value bytes, but a constant store writes
    /// the whole 16-byte pattern.  Pointers are target-sized (4 bytes on
    /// i686), never the host's 8.  `None` for `Void`.
    pub fn access_extent(&self) -> Option<(i64, i64)> {
        let exact = |n: usize| Some((n as i64, n as i64));
        match self {
            IrType::Void => None,
            IrType::F128 if !target_long_double_is_f128() => Some((10, 16)),
            IrType::F128 => exact(16),
            _ => exact(self.size()),
        }
    }

    pub fn size(&self) -> usize {
        match self {
            IrType::I8 | IrType::U8 => 1,
            IrType::I16 | IrType::U16 => 2,
            IrType::I32 | IrType::U32 => 4,
            IrType::I64 | IrType::U64 => 8,
            IrType::Ptr => target_ptr_size(),
            IrType::I128 | IrType::U128 => 16,
            IrType::F32 => 4,
            IrType::F64 => 8,
            IrType::D32 => 4,
            IrType::D64 => 8,
            // i686: F128 (long double) is 12 bytes (80-bit x87 + 2 bytes padding)
            // LP64: F128 is 16 bytes
            IrType::F128 => {
                if target_is_32bit() {
                    12
                } else {
                    16
                }
            }
            IrType::Void => 0,
        }
    }

    /// Alignment in bytes. On i686, some types have alignment smaller than size
    /// (e.g. F128/long double is 12 bytes but aligned to 4, I64/U64 aligned to 4).
    pub fn align(&self) -> usize {
        if target_is_32bit() {
            match self {
                IrType::Void => 1,
                IrType::I8 | IrType::U8 => 1,
                IrType::I16 | IrType::U16 => 2,
                IrType::I32 | IrType::U32 => 4,
                // i686 System V ABI: long long aligned to 4
                IrType::I64 | IrType::U64 => 4,
                IrType::Ptr => 4,
                // i686: i128 aligned to 4
                IrType::I128 | IrType::U128 => 4,
                IrType::F32 => 4,
                // i686: double aligned to 4
                IrType::F64 => 4,
                // i686: long double (80-bit x87, 12 bytes) aligned to 4
                IrType::F128 => 4,
                // Decimal FP: GCC i386 aligns _Decimal64 to 8 (unlike the
                // integer long long align-4 rule), _Decimal32 to 4.
                IrType::D32 => 4,
                IrType::D64 => 8,
            }
        } else {
            match self {
                IrType::Void => 1,
                other => other.size().max(1),
            }
        }
    }

    /// Whether this is an unsigned integer type.
    pub fn is_unsigned(&self) -> bool {
        matches!(
            self,
            IrType::U8 | IrType::U16 | IrType::U32 | IrType::U64 | IrType::U128
        )
    }

    /// Whether this is a signed integer type.
    pub fn is_signed(&self) -> bool {
        matches!(
            self,
            IrType::I8 | IrType::I16 | IrType::I32 | IrType::I64 | IrType::I128
        )
    }

    /// Whether this is any integer type (signed or unsigned).
    pub fn is_integer(&self) -> bool {
        self.is_signed() || self.is_unsigned()
    }

    /// Whether this is a floating-point type (F32, F64, or F128).
    /// F128 (long double) is included because at computation level it is treated
    /// as F64 (stored in D registers), with 16-byte storage for ABI correctness.
    pub fn is_float(&self) -> bool {
        matches!(
            self,
            IrType::F32 | IrType::F64 | IrType::F128 | IrType::D32 | IrType::D64
        )
    }

    /// Whether this is a decimal floating-point carrier type (D32/D64).
    /// D128 is carried as U128 (Float128 model) and is not included here.
    pub fn is_decimal(&self) -> bool {
        matches!(self, IrType::D32 | IrType::D64)
    }

    /// Whether this is a long double type (F128).
    /// Long double values are computed as F64 but stored as 16 bytes for ABI correctness.
    pub fn is_long_double(&self) -> bool {
        matches!(self, IrType::F128)
    }

    /// Whether a `Cast` from `from` to `to` is a machine-level no-op whose
    /// result is bit-identical to its source (same size, neither side
    /// floating). This is the ONLY class of cast that consumer-side
    /// materialization fallbacks may resolve through the source value.
    ///
    /// Same-size float<->int casts are VALUE conversions, not bitcasts:
    /// `(long long)3.0` is cvttsd2si (result 3), while the bit pattern of
    /// 3.0 is 0x4008000000000000; `(double)ptr` is cvtsi2sdq, while
    /// `movq %rax,%xmm0` would re-interpret the address bits (the
    /// check_ptr_to_float_cast precedent). The float guard mirrors the
    /// in-repo convention: global_addr_cse.rs, simplify.rs and
    /// generation.rs already refuse to fold float-involving same-size
    /// casts for exactly this reason.
    pub fn cast_is_bitidentical_nop(from: IrType, to: IrType) -> bool {
        from.size() == to.size() && !from.is_float() && !to.is_float()
    }

    /// Whether this is a 128-bit integer type (I128 or U128).
    pub fn is_128bit(&self) -> bool {
        matches!(self, IrType::I128 | IrType::U128)
    }

    /// Get the unsigned counterpart of this type.
    pub fn to_unsigned(self) -> Self {
        match self {
            IrType::I8 => IrType::U8,
            IrType::I16 => IrType::U16,
            IrType::I32 => IrType::U32,
            IrType::I64 => IrType::U64,
            IrType::I128 => IrType::U128,
            other => other,
        }
    }

    /// Truncate an i64 value to this type's width, applying sign/zero extension
    /// back to i64. Used for case-value truncation in switch statements.
    /// Types wider than 32 bits (and non-integer types) return the value unchanged.
    pub fn truncate_i64(&self, val: i64) -> i64 {
        match self {
            IrType::I8 => val as i8 as i64,
            IrType::U8 => val as u8 as i64,
            IrType::I16 => val as i16 as i64,
            IrType::U16 => val as u16 as i64,
            IrType::I32 => val as i32 as i64,
            IrType::U32 => val as u32 as i64,
            _ => val,
        }
    }

    pub fn from_ctype(ct: &CType) -> Self {
        let is_32bit = target_is_32bit();
        match ct {
            CType::Void => IrType::Void,
            CType::Bool => IrType::U8,
            CType::Char | CType::SChar => IrType::I8,
            CType::UChar | CType::PlainUChar => IrType::U8,
            CType::Short => IrType::I16,
            CType::UShort => IrType::U16,
            CType::Int => IrType::I32,
            // An enum is represented as its compatible integer type.
            CType::Enum(e) => IrType::from_ctype(&e.compatible_ctype()),
            CType::UInt => IrType::U32,
            // ILP32: long = 32-bit; LP64: long = 64-bit
            CType::Long => {
                if is_32bit {
                    IrType::I32
                } else {
                    IrType::I64
                }
            }
            CType::ULong => {
                if is_32bit {
                    IrType::U32
                } else {
                    IrType::U64
                }
            }
            CType::LongLong => IrType::I64,
            CType::ULongLong => IrType::U64,
            CType::Int128 => IrType::I128,
            CType::UInt128 => IrType::U128,
            CType::Float => IrType::F32,
            CType::Double => IrType::F64,
            CType::LongDouble => IrType::F128,
            // _Float128 uses the U128 carrier: 16-byte bit-exact moves keep the
            // IEEE binary128 representation intact (the F128 path is x87-based
            // and would round to 80-bit). Arithmetic/comparison/conversion on
            // Float128 is lowered to libgcc soft-float calls at the CType level.
            CType::Float16 | CType::BFloat16 => IrType::U16,
            CType::Float128 => IrType::U128,
            // Decimal floating point (C23): D32/D64 are first-class SSE-class
            // bit containers (ABI-identical to F32/F64); D128 uses the U128
            // carrier with the Float128 SSE ABI flags, exactly matching GCC's
            // x86-64 classification of TDmode as a 16-byte SSE value.
            CType::Decimal32 => IrType::D32,
            CType::Decimal64 => IrType::D64,
            CType::Decimal128 => IrType::U128,
            // Complex types are handled as aggregate (pointer to stack slot)
            CType::ComplexFloat | CType::ComplexDouble | CType::ComplexLongDouble => IrType::Ptr,
            CType::Pointer(_, _) | CType::Array(_, _) | CType::Function(_) | CType::NullPtr => {
                IrType::Ptr
            }
            CType::Struct(_) | CType::Union(_) => IrType::Ptr,
            // Vectors are treated as aggregate types (pointer to stack slot)
            CType::Vector(_, _) | CType::ExtVector(_, _) => IrType::Ptr,
        }
    }
}

#[cfg(test)]
mod cast_nop_tests {
    use super::IrType;

    /// The cast_is_bitidentical_nop predicate is the shared contract for every
    /// consumer-side "materialize through the cast's source" fallback. A wrong
    /// true here is a silent miscompile (bitcast instead of value conversion);
    /// a wrong false is merely a missed optimization that falls back to the
    /// loud HARD GATE. Pin both directions for every same-size type pair that
    /// the IR's lower_cast can emit.
    #[test]
    fn same_size_int_casts_are_noops() {
        // The print_cfs_group_stats shape (PR #306's intended case).
        assert!(IrType::cast_is_bitidentical_nop(IrType::U64, IrType::I64));
        assert!(IrType::cast_is_bitidentical_nop(IrType::I64, IrType::U64));
        assert!(IrType::cast_is_bitidentical_nop(IrType::U32, IrType::I32));
        assert!(IrType::cast_is_bitidentical_nop(IrType::U16, IrType::I16));
        assert!(IrType::cast_is_bitidentical_nop(IrType::U8, IrType::I8));
        // 128-bit integer sign flavors share a 16-byte representation.
        assert!(IrType::cast_is_bitidentical_nop(IrType::U128, IrType::I128));
    }

    #[test]
    fn same_size_float_int_casts_are_value_conversions() {
        // (long long)d / (int)f — cvttsd2si/cvttss2si, NOT bitcasts.
        assert!(!IrType::cast_is_bitidentical_nop(IrType::F64, IrType::I64));
        assert!(!IrType::cast_is_bitidentical_nop(IrType::F32, IrType::I32));
        // (double)ll / (float)i — cvtsi2sdq/cvtsi2ss.
        assert!(!IrType::cast_is_bitidentical_nop(IrType::I64, IrType::F64));
        assert!(!IrType::cast_is_bitidentical_nop(IrType::I32, IrType::F32));
        // (double)ptr — the check_ptr_to_float_cast precedent (8 == 8 bytes).
        assert!(!IrType::cast_is_bitidentical_nop(IrType::Ptr, IrType::F64));
        // U128 carrier -> F128 (both 16 bytes on LP64): soft-float conversion.
        assert!(!IrType::cast_is_bitidentical_nop(
            IrType::U128,
            IrType::F128
        ));
        assert!(!IrType::cast_is_bitidentical_nop(
            IrType::F128,
            IrType::U128
        ));
    }

    #[test]
    fn size_changing_casts_are_never_noops() {
        // fpext/fptrunc and widening/narrowing all change the representation.
        assert!(!IrType::cast_is_bitidentical_nop(IrType::F32, IrType::F64));
        assert!(!IrType::cast_is_bitidentical_nop(IrType::F64, IrType::F32));
        assert!(!IrType::cast_is_bitidentical_nop(IrType::I32, IrType::I64));
        assert!(!IrType::cast_is_bitidentical_nop(IrType::I64, IrType::I32));
        assert!(!IrType::cast_is_bitidentical_nop(IrType::I8, IrType::I32));
        // i686: F128 is 12 bytes there but still a float — excluded by class.
        assert!(!IrType::cast_is_bitidentical_nop(IrType::F128, IrType::F64));
        assert!(!IrType::cast_is_bitidentical_nop(IrType::F64, IrType::F128));
    }
}

#[cfg(test)]
mod record_display_tag_tests {
    use super::{CType, PtrQuals};

    #[test]
    fn strips_the_struct_prefix() {
        assert_eq!(CType::record_display_tag("struct.S"), "S");
    }

    #[test]
    fn strips_the_union_prefix() {
        assert_eq!(CType::record_display_tag("union.U"), "U");
    }

    #[test]
    fn strips_a_variant_suffix() {
        assert_eq!(CType::record_display_tag("struct.S#1"), "S");
        assert_eq!(CType::record_display_tag("union.U#3"), "U");
    }

    #[test]
    fn leaves_an_anonymous_key_alone() {
        // An anonymous record has no tag to show; its key is what a diagnostic
        // quotes, and stripping part of it would print a name that exists
        // nowhere in the source.
        assert_eq!(
            CType::record_display_tag("__anon_struct_4"),
            "__anon_struct_4"
        );
    }

    #[test]
    fn only_the_first_hash_delimits_the_variant() {
        // Documented, not accidental: '#' is the variant marker, so a tag
        // containing one is already ambiguous. Pinned so changing the marker is
        // a deliberate edit here rather than a silent behaviour change.
        assert_eq!(CType::record_display_tag("struct.a#b#2"), "a");
    }

    #[test]
    fn display_of_a_variant_reads_like_its_base_tag() {
        // The user-facing promise: a variant key must never leak '#N' into a
        // diagnostic, or every shadowing scope would print an internal name.
        assert_eq!(CType::Struct("struct.S#1".into()).to_string(), "struct S");
        assert_eq!(CType::Union("union.U#2".into()).to_string(), "union U");
    }

    #[test]
    fn display_spells_array_declarators_like_gcc() {
        // Each expected string is gcc-16.2's spelling of the same type in an
        // `incompatible pointer type` diagnostic.
        let arr = |t: CType, n| CType::Array(Box::new(t), Some(n));
        let ptr = |t: CType, cvr| CType::Pointer(Box::new(t), PtrQuals::GENERIC.with_cvr(cvr));
        let c = Some(PtrQuals::CONST);
        assert_eq!(ptr(arr(CType::Int, 3), None).to_string(), "int (*)[3]");
        assert_eq!(
            ptr(ptr(arr(CType::Int, 3), None), None).to_string(),
            "int (**)[3]"
        );
        assert_eq!(ptr(arr(CType::Int, 3), c).to_string(), "const int (*)[3]");
        assert_eq!(
            ptr(ptr(arr(CType::Int, 3), Some(0)), c).to_string(),
            "int (* const*)[3]"
        );
        assert_eq!(
            ptr(arr(ptr(CType::Int, None), 3), None).to_string(),
            "int * (*)[3]"
        );
        assert_eq!(
            ptr(arr(ptr(CType::Char, Some(0)), 2), c).to_string(),
            "char * const (*)[2]"
        );
        assert_eq!(
            ptr(arr(arr(CType::Int, 4), 3), None).to_string(),
            "int (*)[3][4]"
        );
        assert_eq!(arr(arr(CType::Int, 4), 3).to_string(), "int[3][4]");
        assert_eq!(
            ptr(ptr(ptr(arr(CType::Int, 5), Some(0)), Some(0)), c).to_string(),
            "int (** const*)[5]"
        );
        // Shapes without a pointed-to or nested array keep the plain spelling.
        assert_eq!(arr(CType::Int, 3).to_string(), "int[3]");
        assert_eq!(arr(ptr(CType::Int, None), 3).to_string(), "int *[3]");
        assert_eq!(ptr(ptr(CType::Char, c), None).to_string(), "const char **");
    }
}

#[cfg(test)]
mod cast_predicate_tests {
    use super::IrType;

    const INTS: [IrType; 8] = [
        IrType::I8,
        IrType::U8,
        IrType::I16,
        IrType::U16,
        IrType::I32,
        IrType::U32,
        IrType::I64,
        IrType::U64,
    ];

    /// The type's actual mathematical domain, in i128 so the extremes fit.
    fn domain(ty: IrType) -> (i128, i128) {
        let bits = ty.size() * 8;
        if ty.is_unsigned() {
            (0, (1i128 << bits) - 1)
        } else {
            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        }
    }

    /// Exhaustive: every ordered pair of integer IR types, checked against the
    /// domain-containment MODEL rather than against the implementation. A
    /// future "simplification" of either predicate therefore fails on a
    /// concrete counterexample instead of on a code review.
    #[test]
    fn value_preservation_is_exactly_domain_containment() {
        for from in INTS {
            for to in INTS {
                let (lo, hi) = domain(from);
                let (tlo, thi) = domain(to);
                assert_eq!(
                    from.cast_preserves_integer_value(to),
                    lo >= tlo && hi <= thi,
                    "{from:?} -> {to:?}"
                );
            }
        }
    }

    /// The offset predicate adds EXACTLY the pointer-ring reinterpretations and
    /// nothing else — no accidental extra admissions.
    #[test]
    fn offset_preservation_adds_only_the_pointer_ring_reinterpretation() {
        let ptr = super::target_ptr_size();
        for from in INTS {
            for to in INTS {
                let ring = from.size() == to.size() && (from.size() as usize) >= ptr;
                assert_eq!(
                    from.cast_preserves_offset_value(to),
                    from.cast_preserves_integer_value(to) || ring,
                    "{from:?} -> {to:?}"
                );
            }
        }
    }

    /// The two named defects, as executable assertions.
    #[test]
    fn the_two_reported_miscompile_shapes_stay_rejected() {
        // IVSR-DOMAIN-1: the composed U32 -> I32 -> I64 that sign-extended a
        // value the source meant as an unsigned index.
        assert!(!IrType::U32.cast_preserves_integer_value(IrType::I32));
        assert!(!IrType::U32.cast_preserves_offset_value(IrType::I32));
        assert!(!IrType::I32.cast_preserves_integer_value(IrType::U32));
        assert!(!IrType::I32.cast_preserves_offset_value(IrType::U32));
        // ...while the composition's second step, a genuine signed widening,
        // is admitted by both.
        assert!(IrType::I32.cast_preserves_integer_value(IrType::I64));
        assert!(IrType::U32.cast_preserves_integer_value(IrType::I64));
        // IVSR-WRAP-1: a truncation is never transparent.
        for (from, to) in [
            (IrType::I32, IrType::U8),
            (IrType::U32, IrType::U16),
            (IrType::I64, IrType::I32),
            (IrType::U64, IrType::U32),
        ] {
            assert!(!from.cast_preserves_offset_value(to), "{from:?}->{to:?}");
        }
        // The pointer-ring arm, which is what keeps the ordinary
        // size_t/ptrdiff_t spelling of a 64-bit index reducible.
        let wide = (IrType::U64.size() as usize) >= super::target_ptr_size();
        assert_eq!(IrType::U64.cast_preserves_offset_value(IrType::I64), wide);
        assert_eq!(IrType::I64.cast_preserves_offset_value(IrType::U64), wide);
        assert!(!IrType::U64.cast_preserves_integer_value(IrType::I64));
    }

    /// Non-integer and pointer types are outside both predicates: neither may
    /// claim a float, a long double or a pointer conversion is value-preserving.
    #[test]
    fn non_integer_types_are_never_admitted() {
        for from in INTS {
            for to in [
                IrType::F32,
                IrType::F64,
                IrType::F128,
                IrType::Ptr,
                IrType::Void,
            ] {
                assert!(!from.cast_preserves_integer_value(to), "{from:?}->{to:?}");
                assert!(!from.cast_preserves_offset_value(to), "{from:?}->{to:?}");
                assert!(!to.cast_preserves_integer_value(from), "{to:?}->{from:?}");
                assert!(!to.cast_preserves_offset_value(from), "{to:?}->{from:?}");
            }
        }
        assert!(!IrType::Ptr.cast_preserves_offset_value(IrType::Ptr));
    }
}

#[cfg(test)]
mod ext_vector_selector_tests {
    use super::{CType, ExtVectorLayout, ExtVectorSelectorError, ext_vector_selector_lanes};

    fn lanes(name: &str, n: usize) -> Vec<usize> {
        ext_vector_selector_lanes(name, n).unwrap_or_else(|e| panic!("{name} on {n}: {e:?}"))
    }

    fn ext_vector(elem: CType, n: usize) -> CType {
        let layout = ExtVectorLayout::new(&elem, n).expect("x86 ext-vector layout");
        CType::ExtVector(Box::new(elem), layout)
    }

    /// Expected lanes are Clang 23.1.3 results on int vectors {10, 11, ...}.
    #[test]
    fn accessor_sets_and_numeric_selectors() {
        assert_eq!(lanes("wzyx", 4), [3, 2, 1, 0]);
        assert_eq!(lanes("abgr", 4), [3, 2, 1, 0]);
        assert_eq!(lanes("s3210", 4), [3, 2, 1, 0]);
        assert_eq!(lanes("S0", 4), [0]);
        assert_eq!(lanes("sF0", 16), [15, 0]);
        assert_eq!(lanes("sf0", 16), [15, 0]);
        assert_eq!(lanes("xyzw", 8), [0, 1, 2, 3]);
        assert_eq!(lanes("xx", 2), [0, 0]);
        assert_eq!(lanes("xyzwxyzwxyzwxyzwx", 4).len(), 17);
    }

    #[test]
    fn halving_selectors_round_up_and_may_name_padding() {
        assert_eq!(lanes("hi", 4), [2, 3]);
        assert_eq!(lanes("lo", 4), [0, 1]);
        assert_eq!(lanes("even", 4), [0, 2]);
        assert_eq!(lanes("odd", 8), [1, 3, 5, 7]);
        // I3 {20, 21, 22}: lo = {20, 21}, hi.x = 22 (lane 3 is padding).
        assert_eq!(lanes("lo", 3), [0, 1]);
        assert_eq!(lanes("hi", 3), [2, 3]);
        assert_eq!(lanes("even", 3), [0, 2]);
        assert_eq!(lanes("odd", 3), [1, 3]);
        // I1: lo is the element, hi lane 1 lies outside the object.
        assert_eq!(lanes("lo", 1), [0]);
        assert_eq!(lanes("hi", 1), [1]);
        assert_eq!(lanes("odd", 5), [1, 3, 5]);
    }

    /// Clang 23.1.3's diagnostic for each rejected selector: the range is
    /// the quoted name, at which Clang's caret points.
    #[test]
    fn clang_rejections() {
        use ExtVectorSelectorError::{ExceedsType, IllegalName};
        for (name, n, err) in [
            ("xg", 4, IllegalName(1..2)),
            ("xr", 4, IllegalName(1..2)),
            ("q", 4, IllegalName(0..1)),
            ("s", 4, IllegalName(0..1)),
            ("sx", 4, IllegalName(1..2)),
            ("s1x", 4, IllegalName(2..3)),
            ("x0", 4, IllegalName(1..2)),
            ("hix", 4, IllegalName(0..1)),
            ("Hi", 4, IllegalName(0..1)),
            ("zyq", 4, IllegalName(2..3)),
            ("x\u{e9}", 4, IllegalName(1..3)),
            ("", 4, IllegalName(0..0)),
            ("s4", 4, ExceedsType),
            ("sA", 4, ExceedsType),
            ("w", 3, ExceedsType),
            ("s3", 3, ExceedsType),
            ("y", 1, ExceedsType),
        ] {
            assert_eq!(
                ext_vector_selector_lanes(name, n),
                Err(err),
                "{name} on {n}"
            );
        }
    }

    #[test]
    fn clang_messages() {
        let int4 = ext_vector(CType::Int, 4);
        let int1 = ext_vector(CType::Int, 1);
        let illegal = ExtVectorSelectorError::IllegalName(1..2);
        assert_eq!(
            illegal.message("xr", &int4),
            "illegal vector component name 'r'"
        );
        let exceeds = ExtVectorSelectorError::ExceedsType.message("s4", &int4);
        assert!(exceeds.starts_with("vector component access exceeds type '"));
        assert!(
            exceeds.ends_with("' (vector of 4 'int' values)"),
            "{exceeds}"
        );
        let one = ExtVectorSelectorError::ExceedsType.message("y", &int1);
        assert!(one.ends_with("' (vector of 1 'int' value)"), "{one}");
    }

    /// Clang 23.1.3 rejects every selector on a `_Bool` ext vector, quoting
    /// the whole name: `b.s1`, `b.x`, `b.lo` and invalid ones alike.
    #[test]
    fn bool_vectors_reject_every_selector() {
        let bool8 = ext_vector(CType::Bool, 8);
        for name in ["s1", "x", "lo", "xyzw", "q"] {
            let err = bool8
                .ext_vector_selection(name)
                .expect("a _Bool ext vector is an ext vector")
                .expect_err(name);
            assert_eq!(err, ExtVectorSelectorError::IllegalName(0..name.len()));
            assert_eq!(
                err.message(name, &bool8),
                format!("illegal vector component name '{name}'")
            );
            assert_eq!(bool8.ext_vector_swizzle(name), None);
        }
        let int4 = ext_vector(CType::Int, 4);
        assert_eq!(int4.ext_vector_swizzle("s1"), Some(vec![1]));
        assert_eq!(CType::Int.ext_vector_selection("x"), None);
    }

    #[test]
    fn selection_types() {
        let int3 = ext_vector(CType::Int, 3);
        assert_eq!(int3.ext_vector_swizzle_ctype("x"), Some(CType::Int));
        assert_eq!(
            int3.ext_vector_swizzle_ctype("hi"),
            Some(ext_vector(CType::Int, 2))
        );
        let int1 = ext_vector(CType::Int, 1);
        assert_eq!(int1.ext_vector_swizzle_ctype("lo"), Some(CType::Int));
        assert_eq!(int1.ext_vector_swizzle_ctype("hi"), Some(CType::Int));
        assert_eq!(
            int3.ext_vector_swizzle_ctype("xyzwx"),
            None,
            "w exceeds a three-lane vector"
        );
        let int4 = ext_vector(CType::Int, 4);
        assert_eq!(
            int4.ext_vector_swizzle_ctype("xyzwx"),
            Some(ext_vector(CType::Int, 5))
        );
    }
}

#[cfg(test)]
mod conditional_type_tests {
    use super::{AddressSpace, CType, FunctionType, PtrQuals};

    fn ptr(ct: CType) -> CType {
        CType::Pointer(Box::new(ct), PtrQuals::GENERIC)
    }

    #[test]
    fn pointer_against_any_integer_is_the_pointer() {
        // GCC 16.2: `c ? 1 : p` and `c ? p : 1L` are `char *`, null pointer
        // constant or not; the integer arm's type truncated the pointer.
        let p = ptr(CType::Char);
        for int in [CType::Int, CType::Long, CType::Bool] {
            for npc in [false, true] {
                assert_eq!(
                    CType::conditional_composite_type(
                        Some(int.clone()),
                        Some(p.clone()),
                        npc,
                        false
                    ),
                    Some(p.clone())
                );
                assert_eq!(
                    CType::conditional_composite_type(
                        Some(p.clone()),
                        Some(int.clone()),
                        false,
                        npc
                    ),
                    Some(p.clone())
                );
            }
        }
    }

    #[test]
    fn void_pointer_arm_follows_its_null_pointer_constant_status() {
        let (void_ptr, int_ptr) = (ptr(CType::Void), ptr(CType::Int));
        assert_eq!(
            CType::conditional_composite_type(
                Some(void_ptr.clone()),
                Some(int_ptr.clone()),
                true,
                false
            ),
            Some(int_ptr.clone())
        );
        assert_eq!(
            CType::conditional_composite_type(
                Some(int_ptr.clone()),
                Some(void_ptr.clone()),
                false,
                false
            ),
            Some(void_ptr.clone())
        );
        assert!(CType::conditional_arm_npc_matters(
            Some(&void_ptr),
            Some(&int_ptr)
        ));
        // Only that pair: an integer or a non-void pointer arm, or `void *`
        // against `void *` or an integer, gets the same type either way.
        assert!(!CType::conditional_arm_npc_matters(
            Some(&CType::Int),
            Some(&int_ptr)
        ));
        assert!(!CType::conditional_arm_npc_matters(
            Some(&int_ptr),
            Some(&void_ptr)
        ));
        assert!(!CType::conditional_arm_npc_matters(
            Some(&void_ptr),
            Some(&void_ptr)
        ));
        assert!(!CType::conditional_arm_npc_matters(
            Some(&void_ptr),
            Some(&CType::Int)
        ));
        assert!(!CType::conditional_arm_npc_matters(None, Some(&int_ptr)));
    }

    #[test]
    fn function_pointer_composite_keeps_prototype_from_either_arm() {
        let unprototyped = CType::Function(Box::new(FunctionType {
            return_type: CType::Int,
            // Body parameters on an unprototyped K&R definition are not a
            // prototype and do not constrain calls through its function type.
            params: vec![(CType::Int, Some("body_param".into()))],
            variadic: false,
            has_prototype: false,
        }));
        let prototype = CType::Function(Box::new(FunctionType {
            return_type: CType::Int,
            params: Vec::new(),
            variadic: false,
            has_prototype: true,
        }));
        let (unprototyped, prototype) = (ptr(unprototyped), ptr(prototype));

        for (then_ty, else_ty) in [
            (unprototyped.clone(), prototype.clone()),
            (prototype.clone(), unprototyped.clone()),
        ] {
            assert_eq!(
                CType::conditional_composite_type(Some(then_ty), Some(else_ty), false, false),
                Some(prototype.clone())
            );
        }
    }

    #[test]
    fn compatible_pointer_to_array_composite_keeps_known_bound() {
        let complete = ptr(CType::Array(Box::new(CType::Int), Some(4)));
        let incomplete = ptr(CType::Array(Box::new(CType::Int), None));
        assert_eq!(
            CType::conditional_composite_type(
                Some(incomplete),
                Some(complete.clone()),
                false,
                false
            ),
            Some(complete)
        );
    }

    #[test]
    fn incompatible_object_pointers_give_void_pointer() {
        // GCC 16.2 (-fpermissive): `c ? (char *)0 : ip` is `void *`.
        let void_ptr = ptr(CType::Void);
        let incompatible = [
            (ptr(CType::Char), ptr(CType::Int)),
            (ptr(CType::Long), ptr(CType::LongLong)),
            (
                ptr(CType::Struct("struct.A".into())),
                ptr(CType::Struct("struct.B".into())),
            ),
            (ptr(CType::Struct("struct.A".into())), ptr(CType::Int)),
            (ptr(ptr(CType::Char)), ptr(ptr(CType::Int))),
        ];
        for (a, b) in incompatible {
            assert_eq!(
                CType::conditional_composite_type(Some(a), Some(b), false, false),
                Some(void_ptr.clone())
            );
        }
        // Same, or not provably incompatible from `CType` alone: unchanged.
        let kept = [
            (ptr(CType::Int), ptr(CType::Int)),
            (
                ptr(CType::Struct("struct.A".into())),
                ptr(CType::Struct("struct.A".into())),
            ),
            (
                ptr(CType::Array(Box::new(CType::Int), Some(3))),
                ptr(CType::Array(Box::new(CType::Int), None)),
            ),
        ];
        for (a, b) in kept {
            assert_eq!(
                CType::conditional_composite_type(Some(a.clone()), Some(b), false, false),
                Some(a)
            );
        }
    }

    #[test]
    fn packed_and_aligned_field_honors_explicit_floor_alignment() {
        let ctx: super::FxHashMap<String, super::RcLayout> = Default::default();
        let fields = vec![
            super::StructField {
                own_cvr: None,
                name: "c".into(),
                ty: CType::Char,
                bit_width: None,
                alignment: None,
                type_align: None,
                is_packed: false,
                base_volatile: false,
                base_const: false,
                object_volatile: false,
                object_const: false,
                counted_by: None,
                pointer_quals: Vec::new(),
            },
            super::StructField {
                own_cvr: None,
                name: "d".into(),
                ty: CType::Double,
                bit_width: None,
                alignment: Some(2),
                type_align: None,
                is_packed: true,
                base_volatile: false,
                base_const: false,
                object_volatile: false,
                object_const: false,
                counted_by: None,
                pointer_quals: Vec::new(),
            },
        ];
        let layout = super::StructLayout::for_struct_with_packing(&fields, None, &ctx, false);
        assert_eq!(layout.align, 2);
        assert_eq!(layout.size, 10);
        assert_eq!(layout.fields[1].offset, 2);
        assert!(layout.user_align);
    }

    #[test]
    fn per_field_packed_bitfield_and_pragma_pack_bitfield_size_match_gcc() {
        let ctx: super::FxHashMap<String, super::RcLayout> = Default::default();
        let packed_bf = vec![super::StructField {
            own_cvr: None,
            name: "a".into(),
            ty: CType::UInt,
            bit_width: Some(4),
            alignment: None,
            type_align: None,
            is_packed: true,
            base_volatile: false,
            base_const: false,
            object_volatile: false,
            object_const: false,
            counted_by: None,
            pointer_quals: Vec::new(),
        }];
        let l1 = super::StructLayout::for_struct_with_packing(&packed_bf, None, &ctx, false);
        assert_eq!(l1.align, 1);
        assert_eq!(l1.size, 1);

        let pack2_bf = vec![super::StructField {
            own_cvr: None,
            name: "a".into(),
            ty: CType::UInt,
            bit_width: Some(4),
            alignment: None,
            type_align: None,
            is_packed: false,
            base_volatile: false,
            base_const: false,
            object_volatile: false,
            object_const: false,
            counted_by: None,
            pointer_quals: Vec::new(),
        }];
        let l2 = super::StructLayout::for_struct_with_packing(&pack2_bf, Some(2), &ctx, false);
        assert_eq!(l2.align, 2);
        assert_eq!(l2.size, 2);
    }
}

#[cfg(test)]
mod pointer_qualifier_contract_tests {
    use super::*;
    use std::hash::{Hash, Hasher};

    fn pointer(cvr: Option<u8>) -> CType {
        CType::Pointer(Box::new(CType::Int), PtrQuals::GENERIC.with_cvr(cvr))
    }

    #[test]
    fn representation_identity_and_semantic_conflicts_are_distinct() {
        let records: Vec<_> = std::iter::once(None).chain((0..8).map(Some)).collect();
        for &a in &records {
            for &b in &records {
                let aq = PtrQuals::GENERIC.with_cvr(a);
                let bq = PtrQuals::GENERIC.with_cvr(b);
                assert_eq!(aq, bq);
                let hash = |q: PtrQuals| {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    q.hash(&mut h);
                    h.finish()
                };
                assert_eq!(hash(aq), hash(bq));
                let conflict = matches!((a, b), (Some(a), Some(b)) if a != b);
                assert_eq!(aq.has_known_cvr_conflict(bq), conflict);
                assert_eq!(ctypes_compatible(&pointer(a), &pointer(b)), !conflict);
            }
        }
        assert_ne!(PtrQuals::GENERIC, PtrQuals::of(AddressSpace::SegGs));
        assert_eq!(PtrQuals::GENERIC.cvr(), None);
        assert_eq!(PtrQuals::GENERIC.with_cvr(Some(0)).cvr(), Some(0));
    }

    #[test]
    fn conditional_joins_all_known_immediate_qualifiers_without_guessing() {
        let records: Vec<_> = std::iter::once(None).chain((0..8).map(Some)).collect();
        for &a in &records {
            for &b in &records {
                let result = CType::conditional_composite_type(
                    Some(pointer(a)),
                    Some(pointer(b)),
                    false,
                    false,
                )
                .unwrap();
                let CType::Pointer(_, quals) = result else {
                    panic!("pointer expected")
                };
                assert_eq!(quals.cvr(), a.zip(b).map(|(a, b)| a | b));
            }
        }
    }

    #[test]
    fn function_qualification_is_significant_but_parameter_names_are_not() {
        let function = |param, name: &str, ret| {
            CType::Function(Box::new(FunctionType {
                return_type: ret,
                params: vec![(param, Some(name.into()))],
                variadic: false,
                has_prototype: true,
            }))
        };
        let plain = function(pointer(Some(0)), "x", CType::Void);
        let renamed = function(pointer(Some(0)), "y", CType::Void);
        let qualified = function(pointer(Some(PtrQuals::CONST)), "x", CType::Void);
        assert!(ctypes_compatible(&plain, &renamed));
        assert!(!recorded_qualifiers_conflict(&plain, &renamed));
        assert!(!ctypes_compatible(&plain, &qualified));
        assert!(recorded_qualifiers_conflict(&plain, &qualified));
        let ret_a = function(CType::Int, "x", pointer(Some(0)));
        let ret_b = function(CType::Int, "x", pointer(Some(PtrQuals::CONST)));
        assert!(recorded_qualifiers_conflict(&ret_a, &ret_b));
        assert!(!ctypes_compatible(&ret_a, &ret_b));
    }
}
