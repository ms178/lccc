//! SysV AMD64 psABI §3.2.3 classification of by-value aggregates.
//!
//! This is the single authority for how a struct, union or GNU vector
//! (`vector_size`) is passed and returned on x86-64. It mirrors GCC's
//! `classify_argument` / `merge_classes` (gcc/config/i386/i386.cc), because
//! GCC defines the convention for its own extensions (`vector_size`,
//! `_Float128`, `__int128`, packed records) and is the interop oracle the
//! regression gate `tests/regression/check_sysv_aggregate_abi.py` checks
//! against in both call directions.
//!
//! Structure, in GCC's terms:
//!
//! * Every leaf is classified by its machine mode: the classes it produces,
//!   and the alignment its bit offset must satisfy. A leaf at an offset that
//!   is not a multiple of its mode alignment ("misaligned field", e.g. an
//!   `int` at byte 1 of a packed record) makes the whole object MEMORY.
//! * A record merges each field's classes into the eightbytes the field
//!   starts in (`merge`), then applies the post-merge cleanup: any MEMORY
//!   eightbyte makes the object MEMORY, a stray SSEUP decays to SSE, and an
//!   X87UP not preceded by X87 makes the object MEMORY. GCC runs the cleanup
//!   at every aggregate level; so does this.
//! * An array classifies its first element and repeats that pattern over its
//!   eightbytes (GCC does not re-check the alignment of later elements).
//! * Objects larger than two eightbytes are MEMORY (the >16-byte SSE/SSEUP
//!   form only exists for AVX vector arguments, which LCCC does not model).
//!
//! Only the *location* classes are modelled. GCC's INTEGERSI/SSESF/SSEDF
//! refinements choose the register mode within the same register and do not
//! change where a value travels.

use super::types::{CType, EightbyteClass, StructLayout, StructLayoutProvider};

/// psABI classes, including those that only exist during classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    NoClass,
    Integer,
    Sse,
    SseUp,
    X87,
    X87Up,
    ComplexX87,
    Memory,
}

/// GCC `merge_classes`, restricted to location classes.
fn merge(a: Class, b: Class) -> Class {
    use Class::*;
    if a == b {
        return a;
    }
    match (a, b) {
        (NoClass, x) | (x, NoClass) => x,
        (Memory, _) | (_, Memory) => Memory,
        (Integer, _) | (_, Integer) => Integer,
        (X87 | X87Up | ComplexX87, _) | (_, X87 | X87Up | ComplexX87) => Memory,
        _ => Sse,
    }
}

/// How a by-value aggregate travels across a call on x86-64.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SysvPassing {
    /// In registers. One or two eightbyte classes drawn from `Integer`,
    /// `Sse` and `NoClass`, or exactly `[Sse, SseUp]` for a 16-byte value
    /// carried in ONE `%xmm` register.
    Regs(Vec<EightbyteClass>),
    /// X87 + X87UP (a lone `long double` member): passed in memory, returned
    /// in `%st(0)`.
    X87,
    /// MEMORY: passed on the stack, returned through a hidden pointer.
    Memory,
}

impl SysvPassing {
    /// Classes for an argument position: anything not passed in registers
    /// is `[Memory]` (an X87 aggregate is MEMORY as an argument).
    pub fn arg_classes(&self) -> Vec<EightbyteClass> {
        match self {
            SysvPassing::Regs(c) => c.clone(),
            SysvPassing::X87 | SysvPassing::Memory => vec![EightbyteClass::Memory],
        }
    }

    /// SSE + SSEUP: the whole 16-byte value in ONE `%xmm` register.
    pub fn one_xmm(&self) -> bool {
        matches!(self, SysvPassing::Regs(c) if c[..] == [EightbyteClass::Sse, EightbyteClass::SseUp])
    }
}

/// Classify `ty` for by-value passing. `None` for types that are not
/// aggregates (scalars have their own conventions) and for incomplete types.
pub fn classify(ty: &CType, ctx: &dyn StructLayoutProvider) -> Option<SysvPassing> {
    if !matches!(ty, CType::Struct(_) | CType::Union(_)) && !ty.is_vector() {
        return None;
    }
    if let CType::Struct(key) | CType::Union(key) = ty {
        ctx.get_struct_layout(key)?;
    }
    Some(finish(classify_at(ty, 0, ctx)))
}

/// The return-position convention: `classify`. Prior revisions special-cased
/// double1 here (\"MEMORY arg, SSE return\"), but ext_vector_leaf already returns
/// `[Sse]` for every 8-byte ext_vector and (empirically, against Clang 23.1.3)
/// both arg and return for one-lane doubles and floats ride the SSE eightbyte
/// uniformly — GCC 16.2's GNU `vector_size` V1SF/V1DF continues to use BLKmode
/// (see the CType::Vector arm below), so the two type spellings keep their
/// distinct ABIs. A special MEMORY-arg/SSE-return split here forced every
/// double1/float1 parameter through a stack home and wasted 24 bytes of frame
/// per call on Raptor Lake.
pub fn classify_return(ty: &CType, ctx: &dyn StructLayoutProvider) -> Option<SysvPassing> {
    classify(ty, ctx)
}

/// Classify a record by its layout (for callers holding a layout, not a type).
pub fn classify_layout(layout: &StructLayout, ctx: &dyn StructLayoutProvider) -> SysvPassing {
    let words = layout.size.div_ceil(8);
    let classes = match words {
        0 => Some(vec![Class::NoClass]),
        1 | 2 => classify_record(layout, words, 0, ctx),
        _ => None,
    };
    finish(classes)
}

/// Map top-level classes (`None` = MEMORY) to the passing convention.
fn finish(classes: Option<Vec<Class>>) -> SysvPassing {
    let Some(classes) = classes else {
        return SysvPassing::Memory;
    };
    use Class::*;
    let out = match classes[..] {
        [X87, X87Up] => return SysvPassing::X87,
        _ if classes
            .iter()
            .any(|c| matches!(c, X87 | X87Up | ComplexX87 | Memory)) =>
        {
            return SysvPassing::Memory;
        }
        _ => classes
            .iter()
            .map(|c| match c {
                NoClass => EightbyteClass::NoClass,
                Integer => EightbyteClass::Integer,
                Sse => EightbyteClass::Sse,
                SseUp => EightbyteClass::SseUp,
                X87 | X87Up | ComplexX87 | Memory => unreachable!("filtered above"),
            })
            .collect(),
    };
    SysvPassing::Regs(out)
}

/// Byte size of `ty` with record layouts resolved through `ctx`.
fn size_of(ty: &CType, ctx: &dyn StructLayoutProvider) -> usize {
    match ty {
        CType::Struct(key) | CType::Union(key) => ctx.get_struct_layout(key).map_or(0, |l| l.size),
        _ => ty.size_ctx(ctx),
    }
}

/// Classes of `ty` placed at `bit_offset` within the outermost object, as
/// GCC's `classify_argument`: index 0 is the eightbyte containing the first
/// byte of `ty`. `None` means MEMORY.
fn classify_at(
    ty: &CType,
    bit_offset: usize,
    ctx: &dyn StructLayoutProvider,
) -> Option<Vec<Class>> {
    let bytes = size_of(ty, ctx);
    let words = (bytes + (bit_offset % 64) / 8).div_ceil(8);
    if words == 0 {
        // Zero-sized records and arrays occupy nothing.
        return Some(vec![Class::NoClass]);
    }
    if words > 2 {
        return None;
    }
    match ty {
        CType::Struct(key) | CType::Union(key) => {
            classify_record(ctx.get_struct_layout(key)?, words, bit_offset, ctx)
        }
        CType::Array(elem, _) => {
            let sub = classify_at(elem, bit_offset, ctx)?;
            Some((0..words).map(|i| sub[i % sub.len()]).collect())
        }
        _ => {
            let (classes, align_bits) = leaf(ty)?;
            if bit_offset % align_bits != 0 {
                return None; // misaligned field
            }
            if matches!(ty, CType::ComplexFloat) && bit_offset % 64 != 0 {
                // GCC >= 4.4: an SCmode field that does not start an
                // eightbyte straddles two (`struct { int i; _Complex float
                // c; }` puts the imaginary part in the second): SSE, SSESF.
                return Some(vec![Class::Sse, Class::Sse]);
            }
            Some(classes)
        }
    }
}

/// A record (struct or union) of `words` eightbytes at `bit_offset`.
fn classify_record(
    layout: &StructLayout,
    words: usize,
    bit_offset: usize,
    ctx: &dyn StructLayoutProvider,
) -> Option<Vec<Class>> {
    let mut classes = vec![Class::NoClass; words];
    for f in &layout.fields {
        let field_bits = f.offset * 8 + f.bit_offset.unwrap_or(0) as usize;
        if let Some(width) = f.bit_width {
            if width == 0 {
                continue;
            }
            // Bit-fields are INTEGER over every eightbyte they touch
            // and are exempt from the misalignment rule.
            let start = (field_bits + bit_offset % 64) / 64;
            let end = (field_bits + bit_offset % 64 + width as usize).div_ceil(64);
            for c in classes.iter_mut().take(end).skip(start) {
                *c = merge(Class::Integer, *c);
            }
            continue;
        }
        if matches!(f.ty, CType::Array(_, None)) {
            continue; // flexible array member: no storage
        }
        let sub = classify_at(&f.ty, (field_bits + bit_offset) % 512, ctx)?;
        let pos = (field_bits + bit_offset % 64) / 64;
        for (i, s) in sub.iter().enumerate() {
            if let Some(c) = classes.get_mut(i + pos) {
                *c = merge(*s, *c);
            }
        }
    }
    cleanup(classes)
}

/// GCC's post-merge cleanup for one aggregate level.
fn cleanup(mut classes: Vec<Class>) -> Option<Vec<Class>> {
    for i in 0..classes.len() {
        let prev = if i == 0 {
            Class::NoClass
        } else {
            classes[i - 1]
        };
        match classes[i] {
            Class::Memory => return None,
            Class::SseUp if !matches!(prev, Class::Sse | Class::SseUp) => classes[i] = Class::Sse,
            Class::X87Up if prev != Class::X87 => return None,
            _ => {}
        }
    }
    Some(classes)
}

/// Mode classification of a non-aggregate leaf: its classes and the
/// alignment (in bits) its offset must satisfy. `None` for leaves GCC gives
/// BLKmode (MEMORY): one-lane floating vectors, which have no vector mode.
fn leaf(ty: &CType) -> Option<(Vec<Class>, usize)> {
    use Class::*;
    Some(match ty {
        CType::Float | CType::Decimal32 => (vec![Sse], 32),
        CType::Double | CType::Decimal64 => (vec![Sse], 64),
        CType::LongDouble => (vec![X87, X87Up], 128),
        CType::Float128 | CType::Decimal128 => (vec![Sse, SseUp], 128),
        // SCmode is ONE SSE class when it starts an eightbyte (see
        // `classify_at` for the straddling case); DCmode is two SSE eightbytes; XCmode is COMPLEX_X87.
        CType::ComplexFloat => (vec![Sse], 32),
        CType::ComplexDouble => (vec![Sse, Sse], 64),
        CType::ComplexLongDouble => (vec![ComplexX87], 128),
        CType::Vector(elem, total) => {
            let lanes_fp = matches!(**elem, CType::Float | CType::Double | CType::LongDouble);
            // A single-lane FP vector (V1SFmode/V1DFmode = vector_size(4/8)
            // of float/double) has no native vector machine mode in GCC and
            // falls through to BLKmode (MEMORY). GCC 16.2 is empirically
            // consistent with this: `vector_size(8) double` returns by hidden
            // pointer. Keep this behavior to preserve GNU-vector interop;
            // Clang's ext_vector_type(1) double is handled in
            // `ext_vector_leaf` and uses the SSE register.
            if lanes_fp && elem.size() == *total {
                return None; // V1SF / V1DF: no vector mode, BLKmode
            }
            match *total {
                16 => (vec![Sse, SseUp], 128),
                8 => (vec![Sse], 64),
                // Sub-eightbyte vectors: integer vector modes (or the
                // same-sized integer mode) take the INTEGER default.
                1 | 2 | 4 => (vec![Integer], *total * 8),
                _ => return None,
            }
        }
        CType::ExtVector(elem, layout) => return ext_vector_leaf(elem, layout, ty.size()),
        CType::Int128 | CType::UInt128 => (vec![Integer, Integer], 128),
        // Integers, _Bool, enums, pointers: INTEGER, aligned to their size.
        _ => {
            let size = ty.size().max(1);
            (vec![Integer], size * 8)
        }
    })
}

/// Clang's x86-64 classification of an `ext_vector_type` (measured against
/// Clang 23.1.3, including bit-packed `_Bool` vectors, padded three-lane
/// shapes and members): 1, 2 and 4 bytes are INTEGER; 8 bytes are SSE except
/// one `double` lane (MEMORY); 16 bytes are SSE + SSEUP (except a 128-lane
/// `_Bool` vector, MEMORY); larger is MEMORY. A
/// `long double` lane is the X87 pair, so a one-lane vector behaves exactly
/// like `long double` (MEMORY argument, `%st(0)` return).
fn ext_vector_leaf(
    elem: &CType,
    layout: &super::types::ExtVectorLayout,
    size: usize,
) -> Option<(Vec<Class>, usize)> {
    use Class::*;
    if matches!(elem, CType::LongDouble) {
        return (layout.lanes == 1).then(|| (vec![X87, X87Up], 128));
    }
    if layout.bitpacked_bool && layout.lanes > 64 {
        // A 16-byte `_Bool` vector returns through sret (its argument form
        // is rejected, see `x86_64_ext_vector_unsupported`).
        return None;
    }
    Some(match size {
        1 | 2 => (vec![Integer], size * 8),
        // Clang 23.1.3 empirical measurement: one-lane ext_vector_type of
        // float (f32x1 / <1 x float>, size 4) and double (f64x1 / <1 x
        // double>, size 8) pass AND return in the low bits of an SSE
        // register (%xmm0 for the first arg/return), exactly like their
        // scalar base type. Sub-4-byte integer one-lanes and the 4-byte
        // integer one-lane use INTEGER (the psABI default for sub-8 byte
        // vectors in the Integer class). An 8-byte one-lane integer vector
        // (u64x1 / i64x1) also classifies as INTEGER (consistent with GCC
        // 16.2's GNU-vector u64x1 ABI and the psABI default for non-FP
        // aggregates); the earlier blanket SSE classification here forced
        // u64x1 returns through %xmm0 where only the low 32 bits of the
        // NaN-boxed double image survived (yielding 0x8000000000000000).
        // Multi-lane small vectors (<=4 bytes) keep their integer carrier
        // per psABI; multi-lane 8-byte vectors (e.g. u16x4, u32x2) stay SSE.
        4 if layout.lanes == 1 && matches!(elem, CType::Float) => (vec![Sse], 32),
        4 => (vec![Integer], 32),
        8 if layout.lanes == 1 && matches!(elem, CType::Double) => (vec![Sse], 64),
        8 if layout.lanes == 1 => (vec![Integer], 64),
        8 => (vec![Sse], 64),
        16 => (vec![Sse, SseUp], 128),
        _ => return None,
    })
}
