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

/// Get the target pointer size for the current thread.
pub fn target_ptr_size() -> usize {
    TARGET_PTR_SIZE.with(|c| c.get())
}

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

/// System V AMD64 ABI classification for a single 8-byte "eightbyte" of a struct.
/// Used to determine whether a struct field group should be passed in GP or SSE registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EightbyteClass {
    /// No fields occupy this eightbyte yet (initial state during classification).
    NoClass,
    /// All fields in this eightbyte are float/double -> pass in xmm register.
    Sse,
    /// At least one non-float field in this eightbyte -> pass in GP register.
    Integer,
}

impl EightbyteClass {
    /// Merge two eightbyte classifications per SysV ABI rules:
    /// - NoClass + X = X
    /// - Integer + anything = Integer
    /// - SSE + SSE = SSE
    pub fn merge(self, other: EightbyteClass) -> EightbyteClass {
        match (self, other) {
            (EightbyteClass::NoClass, x) | (x, EightbyteClass::NoClass) => x,
            (EightbyteClass::Integer, _) | (_, EightbyteClass::Integer) => EightbyteClass::Integer,
            (EightbyteClass::Sse, EightbyteClass::Sse) => EightbyteClass::Sse,
        }
    }
}

/// Classification of a struct for RISC-V LP64D hardware floating-point calling convention.
///
/// The psABI specifies that small structs with specific field patterns should be
/// passed in FP registers rather than GP registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiscvFloatClass {
    /// Struct with exactly one float/double member, no other data members.
    /// Passed in a single FP register (fa0-fa7).
    OneFloat { is_double: bool },
    /// Struct with exactly two float/double members, no other data members.
    /// Passed in two FP registers (fa0-fa7).
    TwoFloats {
        lo_is_double: bool,
        hi_is_double: bool,
    },
    /// Struct with one float/double + one integer, float comes first in memory.
    /// Float in FP register, integer in GP register.
    FloatAndInt {
        float_is_double: bool,
        float_offset: usize,
        int_offset: usize,
        int_size: usize,
    },
    /// Struct with one integer + one float/double, integer comes first in memory.
    /// Integer in GP register, float in FP register.
    IntAndFloat {
        float_is_double: bool,
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

/// Represents C types in the compiler.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CType {
    Void,
    Bool,
    Char,
    UChar,
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
    Pointer(Box<CType>, AddressSpace),
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
    /// Stores (element_type, total_size_in_bytes).
    /// E.g., `typedef int v4si __attribute__((vector_size(16)))` -> Vector(Int, 16)
    /// has 4 elements of type int, total size 16 bytes.
    Vector(Box<CType>, usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionType {
    pub return_type: CType,
    pub params: Vec<(CType, Option<String>)>,
    pub variadic: bool,
}

/// StructField describes a single field in a struct or union declaration.
/// Used during construction of StructLayout.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructField {
    pub name: String,
    pub ty: CType,
    pub bit_width: Option<u32>,
    /// Per-field alignment override from _Alignas(N) or __attribute__((aligned(N))).
    /// When set, this overrides the natural alignment of the field's type.
    pub alignment: Option<usize>,
    /// Per-field __attribute__((packed)) - forces this field's alignment to 1.
    pub is_packed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumType {
    pub name: Option<String>,
    pub variants: Vec<(String, i64)>,
    /// When true (__attribute__((packed))), the enum uses the smallest
    /// integer type that can represent all variant values.
    pub is_packed: bool,
}

impl EnumType {
    /// Returns the size (and alignment) in bytes for this enum.
    /// Non-packed enums are always 4 bytes (int). Packed enums use the
    /// smallest integer type that can represent all variant values.
    pub fn packed_size(&self) -> usize {
        if !self.is_packed {
            // Non-packed enum: 4 bytes if all values fit in int or unsigned int,
            // otherwise 8 bytes (GCC extension for values exceeding 32-bit range).
            // Values like `1U << 31` (0x80000000) fit in unsigned int (u32) even
            // though they exceed signed int (i32) range.
            let exceeds_32bit = self
                .variants
                .iter()
                .any(|(_, v)| *v > u32::MAX as i64 || *v < i32::MIN as i64);
            if exceeds_32bit {
                return 8;
            }
            return 4;
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
        }
    }

    /// Compute the effective alignment and size for a field.
    fn compute_field_alignment(
        &mut self,
        field: &StructField,
        max_field_align: Option<usize>,
        ctx: &dyn StructLayoutProvider,
    ) -> (usize, usize) {
        let natural_align = field.ty.align_ctx(ctx);
        let field_align = if field.is_packed {
            // Per-field __attribute__((packed)) forces alignment to 1
            1
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
            if field_align > 0 {
                field_align
            } else {
                1
            }
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
        let (sso, sso_storage_ty, field_bit_in_storage, storage_ctype) = if self.reverse_sso {
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
            if self.is_packed_1 {
                let total_bits = (self.bf_unit_offset * 8) as u32 + self.bf_bit_pos;
                self.offset = (total_bits as usize).div_ceil(8);
            } else {
                self.offset = self.bf_unit_offset + self.bf_unit_size;
            }
        }

        let size = align_up(self.offset, self.max_align);
        StructLayout {
            fields: self.field_layouts,
            size,
            align: self.max_align,
            is_union: false,
            is_transparent_union: false,
            reverse_sso: self.reverse_sso,
        }
    }
}

impl StructLayout {
    /// Create an empty StructLayout (zero-size, no fields).
    /// Used as a fallback when a struct/union layout is not found.
    pub fn empty() -> Self {
        StructLayout {
            fields: Vec::new(),
            size: 0,
            align: 1,
            is_union: false,
            is_transparent_union: false,
            reverse_sso: false,
        }
    }

    /// Create an empty union StructLayout (zero-size, no fields, is_union=true).
    pub fn empty_union() -> Self {
        StructLayout {
            fields: Vec::new(),
            size: 0,
            align: 1,
            is_union: true,
            is_transparent_union: false,
            reverse_sso: false,
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
}

impl StructLayout {
    /// Return the smallest integer CType that can hold at least `needed_bytes` bytes.
    /// Preserves signedness.
    /// Uses LongLong (always 8 bytes) instead of Long for >4 bytes, because on
    /// i686 (ILP32) Long is only 4 bytes and would truncate 64-bit bitfields.
    fn smallest_int_ctype_for_bytes(needed_bytes: usize, is_signed: bool) -> CType {
        if is_signed {
            match needed_bytes {
                0..=1 => CType::Char,
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
                } else if b.is_packed_1 {
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
        let mut field_layouts = Vec::with_capacity(fields.len());

        for field in fields {
            let natural_align = field.ty.align_ctx(ctx);
            // Per-field alignment handling
            let field_align = if field.is_packed {
                // Per-field __attribute__((packed)) forces alignment to 1
                1
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
            fields: field_layouts,
            size,
            align: max_align,
            is_union: true,
            is_transparent_union: false,
            reverse_sso,
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

    /// Classify each 8-byte "eightbyte" of this struct/union per the System V AMD64 ABI.
    ///
    /// Returns a vector with one entry per eightbyte (so 1 entry for size <= 8, 2 for size <= 16).
    /// Each entry is `EightbyteClass::Sse` if all fields in that eightbyte are float/double,
    /// or `EightbyteClass::Integer` otherwise.
    ///
    /// Structs > 16 bytes get MEMORY class (empty vec, meaning pass on stack).
    /// Structs containing unaligned fields or bitfields spanning eightbyte boundaries
    /// are conservatively classified as INTEGER.
    pub fn classify_sysv_eightbytes(&self, ctx: &dyn StructLayoutProvider) -> Vec<EightbyteClass> {
        // Structs > 16 bytes -> MEMORY class
        if self.size > 16 || self.size == 0 {
            return Vec::new();
        }

        let n_eightbytes = if self.size <= 8 { 1 } else { 2 };
        // Start with NO_CLASS (uninitialized), then merge field classifications
        let mut classes = vec![EightbyteClass::NoClass; n_eightbytes];

        for field in &self.fields {
            // Skip zero-width bitfields (padding only)
            if field.bit_width == Some(0) {
                continue;
            }
            // Bitfields complicate classification -- conservatively treat as INTEGER
            if field.bit_width.is_some() {
                let eb_idx = field.offset / 8;
                if eb_idx < n_eightbytes {
                    classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Integer);
                }
                continue;
            }

            Self::classify_field_type(&field.ty, field.offset, &mut classes, n_eightbytes, ctx);
        }

        // Replace any remaining NoClass with Integer (e.g., padding-only eightbytes)
        for c in &mut classes {
            if *c == EightbyteClass::NoClass {
                *c = EightbyteClass::Integer;
            }
        }

        classes
    }

    /// Recursively classify a field's type into the eightbyte slots it occupies.
    fn classify_field_type(
        ty: &CType,
        base_offset: usize,
        classes: &mut [EightbyteClass],
        n_eightbytes: usize,
        ctx: &dyn StructLayoutProvider,
    ) {
        match ty {
            CType::Float => {
                let eb_idx = base_offset / 8;
                if eb_idx < n_eightbytes {
                    classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Sse);
                }
            }
            CType::Double => {
                let eb_idx = base_offset / 8;
                if eb_idx < n_eightbytes {
                    classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Sse);
                }
            }
            // Decimal FP members classify like the binary floats of equal
            // width (GCC SysV: _Decimal32/_Decimal64 are SSE, _Decimal128
            // occupies two SSE eightbytes).
            CType::Decimal32 | CType::Decimal64 => {
                let eb_idx = base_offset / 8;
                if eb_idx < n_eightbytes {
                    classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Sse);
                }
            }
            CType::Decimal128 => {
                let eb_idx = base_offset / 8;
                if eb_idx + 1 < n_eightbytes {
                    classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Sse);
                    classes[eb_idx + 1] = classes[eb_idx + 1].merge(EightbyteClass::Sse);
                } else if eb_idx < n_eightbytes {
                    classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Sse);
                }
            }
            // Array: classify each element
            CType::Array(elem_ty, Some(count)) => {
                let elem_size = elem_ty.size();
                if elem_size > 0 {
                    for i in 0..*count {
                        Self::classify_field_type(
                            elem_ty,
                            base_offset + i * elem_size,
                            classes,
                            n_eightbytes,
                            ctx,
                        );
                    }
                }
            }
            // Nested struct/union: classify each of its fields
            CType::Struct(key) | CType::Union(key) => {
                if let Some(layout) = ctx.get_struct_layout(key) {
                    for inner_field in &layout.fields {
                        if inner_field.bit_width == Some(0) {
                            continue;
                        }
                        if inner_field.bit_width.is_some() {
                            let eb_idx = (base_offset + inner_field.offset) / 8;
                            if eb_idx < n_eightbytes {
                                classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Integer);
                            }
                            continue;
                        }
                        Self::classify_field_type(
                            &inner_field.ty,
                            base_offset + inner_field.offset,
                            classes,
                            n_eightbytes,
                            ctx,
                        );
                    }
                } else {
                    // Unknown layout: treat conservatively as INTEGER
                    let eb_idx = base_offset / 8;
                    if eb_idx < n_eightbytes {
                        classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Integer);
                    }
                }
            }
            // Vector types: classify element-wise according to the element type.
            // float/double vectors -> SSE, integer vectors -> INTEGER.
            CType::Vector(elem_ty, total_size) => {
                let elem_size = elem_ty.size();
                if elem_size > 0 {
                    let num_elems = total_size / elem_size;
                    for i in 0..num_elems {
                        Self::classify_field_type(
                            elem_ty,
                            base_offset + i * elem_size,
                            classes,
                            n_eightbytes,
                            ctx,
                        );
                    }
                } else {
                    let eb_idx = base_offset / 8;
                    if eb_idx < n_eightbytes {
                        classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Integer);
                    }
                }
            }
            // All other types (integers, pointers, enums, etc.) -> INTEGER
            _ => {
                let eb_idx = base_offset / 8;
                if eb_idx < n_eightbytes {
                    classes[eb_idx] = classes[eb_idx].merge(EightbyteClass::Integer);
                }
            }
        }
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
            (1, 0) => {
                let is_double = float_fields[0].1 == 8;
                Some(RiscvFloatClass::OneFloat { is_double })
            }
            // 2 floats, no int: pass both in FP regs
            (2, 0) => {
                let lo_is_double = float_fields[0].1 == 8;
                let hi_is_double = float_fields[1].1 == 8;
                Some(RiscvFloatClass::TwoFloats {
                    lo_is_double,
                    hi_is_double,
                })
            }
            // 1 float + 1 int: pass float in FP reg, int in GP reg
            (1, 1) => {
                let float_is_double = float_fields[0].1 == 8;
                let int_size = int_fields[0].1;
                // Integer field must fit in one XLEN register
                if int_size > xlen {
                    return None;
                }
                // Determine which comes first in memory layout
                if float_fields[0].0 < int_fields[0].0 {
                    Some(RiscvFloatClass::FloatAndInt {
                        float_is_double,
                        float_offset: float_fields[0].0,
                        int_offset: int_fields[0].0,
                        int_size,
                    })
                } else {
                    Some(RiscvFloatClass::IntAndFloat {
                        float_is_double,
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

    /// Look up a field by name, returning full layout info including bitfield details.
    pub fn field_layout(&self, name: &str) -> Option<&StructFieldLayout> {
        self.fields.iter().find(|f| f.name == name)
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

impl std::fmt::Display for CType {
    /// Format a CType as its C-language type name (e.g., `int`, `unsigned long`,
    /// `char *`, `void (*)(int, double)`). Used in compiler diagnostics to show
    /// user-friendly type names instead of the Rust Debug representation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CType::Void => write!(f, "void"),
            CType::Bool => write!(f, "_Bool"),
            CType::Char => write!(f, "char"),
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
            CType::Float128 => write!(f, "_Float128"),
            CType::Decimal32 => write!(f, "_Decimal32"),
            CType::Decimal64 => write!(f, "_Decimal64"),
            CType::Decimal128 => write!(f, "_Decimal128"),
            CType::ComplexFloat => write!(f, "_Complex float"),
            CType::ComplexDouble => write!(f, "_Complex double"),
            CType::ComplexLongDouble => write!(f, "_Complex long double"),
            CType::Pointer(inner, addr_space) => {
                let prefix = match addr_space {
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
                    if ft.variadic {
                        if !ft.params.is_empty() {
                            write!(f, ", ")?;
                        }
                        write!(f, "...")?;
                    }
                    write!(f, ")")
                } else {
                    write!(f, "{}{} *", prefix, inner)
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
                if ft.variadic {
                    if !ft.params.is_empty() {
                        write!(f, ", ")?;
                    }
                    write!(f, "...")?;
                }
                write!(f, ")")
            }
            CType::Struct(name) => {
                // Strip the "struct." prefix if present for cleaner display
                let display_name = name.strip_prefix("struct.").unwrap_or(name);
                if display_name.starts_with("__anon_struct_") {
                    write!(f, "struct <anonymous>")
                } else {
                    write!(f, "struct {}", display_name)
                }
            }
            CType::Union(name) => {
                // Strip the "union." or "struct." prefix if present
                let display_name = name
                    .strip_prefix("union.")
                    .or_else(|| name.strip_prefix("struct."))
                    .unwrap_or(name);
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
pub fn char_is_unsigned() -> bool {
    CHAR_UNSIGNED.load(std::sync::atomic::Ordering::Relaxed)
}

impl CType {
    /// Size in bytes, with struct/union layout lookup via context.
    /// Uses the thread-local target pointer size for target-dependent types
    /// (Long, Pointer, LongDouble).
    pub fn size_ctx(&self, ctx: &dyn StructLayoutProvider) -> usize {
        let ptr_sz = target_ptr_size();
        match self {
            CType::Void => 0,
            CType::Bool | CType::Char | CType::UChar => 1,
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
            CType::Pointer(_, _) => ptr_sz,
            CType::Array(elem, Some(n)) => elem.size_ctx(ctx) * n,
            CType::Array(_, None) => ptr_sz, // incomplete array treated as pointer
            CType::Function(_) => ptr_sz,    // function pointer size
            CType::Struct(key) | CType::Union(key) => {
                ctx.get_struct_layout(key).map(|l| l.size).unwrap_or(0)
            }
            CType::Enum(e) => e.packed_size(),
            CType::Vector(_, total_size) => *total_size,
        }
    }

    /// Alignment in bytes, with struct/union layout lookup via context.
    /// Uses the thread-local target pointer size for target-dependent types.
    pub fn align_ctx(&self, ctx: &dyn StructLayoutProvider) -> usize {
        let ptr_sz = target_ptr_size();
        match self {
            CType::Void => 1,
            CType::Bool | CType::Char | CType::UChar => 1,
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
            CType::Pointer(_, _) => ptr_sz,
            CType::Array(elem, _) => elem.align_ctx(ctx),
            CType::Function(_) => ptr_sz,
            CType::Struct(key) | CType::Union(key) => {
                ctx.get_struct_layout(key).map(|l| l.align).unwrap_or(1)
            }
            CType::Enum(e) => e.packed_size(),
            // GCC caps vector alignment at 16 bytes on x86-64
            CType::Vector(_, total_size) => (*total_size).min(16),
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

    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            CType::Bool
                | CType::Char
                | CType::UChar
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
        matches!(
            self,
            CType::Char | CType::Short | CType::Int | CType::Long | CType::LongLong | CType::Int128
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

    /// Whether this is an arithmetic type (integer, floating-point, or complex).
    pub fn is_arithmetic(&self) -> bool {
        self.is_integer() || self.is_floating() || self.is_complex()
    }

    /// Whether this is a GCC vector extension type.
    pub fn is_vector(&self) -> bool {
        matches!(self, CType::Vector(_, _))
    }

    /// For a vector type, returns (element_type, num_elements).
    /// Returns None for non-vector types.
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
            _ => None,
        }
    }

    /// Apply C integer promotion rules (C11 6.3.1.1):
    /// Types smaller than int are promoted to int (or unsigned int if int cannot
    /// represent all values, but for our targets int is 32-bit so Bool/Char/UChar/
    /// Short/UShort all fit in int).
    pub fn integer_promoted(&self) -> CType {
        match self {
            CType::Bool | CType::Char | CType::UChar | CType::Short | CType::UShort => CType::Int,
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
        matches!(
            self,
            CType::Bool
                | CType::UChar
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
            CType::Char | CType::UChar => 1,
            CType::Short | CType::UShort => 2,
            CType::Int | CType::UInt => 3,
            CType::Enum(e) => {
                // Enum rank follows its underlying type size
                match e.packed_size() {
                    1 => 1,
                    2 => 2,
                    8 => 5, // same rank as long long
                    _ => 3, // int
                }
            }
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

        // Both integer types after promotion: apply conversion rank rules
        let l_rank = l.integer_rank();
        let r_rank = r.integer_rank();
        let l_unsigned = l.is_unsigned();
        let r_unsigned = r.is_unsigned();

        if l_unsigned == r_unsigned {
            if l_rank >= r_rank {
                l
            } else {
                r
            }
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
            CType::Char => CType::UChar,
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
    pub fn conditional_composite_type(
        then_ct: Option<CType>,
        else_ct: Option<CType>,
        then_is_npc: bool,
        else_is_npc: bool,
    ) -> Option<CType> {
        match (then_ct, else_ct) {
            (Some(t), Some(e)) => {
                // Both void
                if matches!(&t, CType::Void) && matches!(&e, CType::Void) {
                    return Some(CType::Void);
                }
                // Both pointers: C11 6.5.15p6 rules
                if let (CType::Pointer(ref inner_t, _), CType::Pointer(ref inner_e, _)) = (&t, &e) {
                    let t_is_void = matches!(inner_t.as_ref(), CType::Void);
                    let e_is_void = matches!(inner_e.as_ref(), CType::Void);
                    if t_is_void && !e_is_void {
                        // then=void*, else=T*: if then is NPC, result is T*; else void*
                        return if then_is_npc { Some(e) } else { Some(t) };
                    }
                    if e_is_void && !t_is_void {
                        // then=T*, else=void*: if else is NPC, result is T*; else void*
                        return if else_is_npc { Some(t) } else { Some(e) };
                    }
                    return Some(t);
                }
                // One pointer, one integer 0 (null pointer constant)
                if let CType::Pointer(..) = &t {
                    if e.is_arithmetic() && else_is_npc {
                        return Some(t);
                    }
                }
                if let CType::Pointer(..) = &e {
                    if t.is_arithmetic() && then_is_npc {
                        return Some(e);
                    }
                }
                // Both arithmetic types: apply usual arithmetic conversions
                if t.is_arithmetic() && e.is_arithmetic() {
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
            CType::Char => IrType::I8,
            CType::UChar => IrType::U8,
            CType::Short => IrType::I16,
            CType::UShort => IrType::U16,
            CType::Int => IrType::I32,
            CType::Enum(e) => {
                // Map enum to IR type based on its computed size.
                // For non-packed enums with 64-bit values, packed_size() returns 8.
                match e.packed_size() {
                    1 => IrType::I8,
                    2 => IrType::I16,
                    8 => {
                        // 64-bit enums: check signedness
                        if e.variants.iter().any(|(_, v)| *v < 0) {
                            IrType::I64
                        } else {
                            IrType::U64
                        }
                    }
                    _ => {
                        // 4-byte enum: unsigned if any value exceeds i32 range
                        // (e.g., 1U << 31 = 0x80000000 fits in u32 but not i32)
                        if e.variants.iter().any(|(_, v)| *v > i32::MAX as i64) {
                            IrType::U32
                        } else {
                            IrType::I32
                        }
                    }
                }
            }
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
            CType::Pointer(_, _) | CType::Array(_, _) | CType::Function(_) => IrType::Ptr,
            CType::Struct(_) | CType::Union(_) => IrType::Ptr,
            // Vectors are treated as aggregate types (pointer to stack slot)
            CType::Vector(_, _) => IrType::Ptr,
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
