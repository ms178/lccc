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
//! * Objects larger than two eightbytes are MEMORY, except the AVX vector
//!   form: under `-mavx` (`-mavx512f`) an object of up to four (eight)
//!   eightbytes whose classes are exactly SSE followed by SSEUPs travels in
//!   ONE `%ymm` (`%zmm`) register — psABI §3.2.3 rule (c) of the
//!   post-merger cleanup, GCC's `TARGET_AVX` / `TARGET_AVX512F` arms of
//!   `classify_argument`. The width ceiling is
//!   `types::target_vector_abi_bytes()`, which the driver derives from the
//!   TU's requested ISA exactly like `__AVX__`.
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
    /// Two consecutive `long double` lanes (Clang's `<2 x x86_fp80>`, the
    /// two-lane `long double` ext vector): MEMORY as an argument (32-byte
    /// `byval`), but Clang hands the return to LLVM "direct"
    /// (`getIndirectReturnResult` for a non-aggregate), where the vector is
    /// scalarized and the x86 return convention assigns the two `f80`s to
    /// `%st(0)` and `%st(1)` — the `_Complex long double` register pair.
    X87Pair,
    /// MEMORY: passed on the stack, returned through a hidden pointer.
    Memory,
}

impl SysvPassing {
    /// Classes for an argument position: anything not passed in registers
    /// is `[Memory]` (an X87 aggregate is MEMORY as an argument).
    pub fn arg_classes(&self) -> Vec<EightbyteClass> {
        match self {
            SysvPassing::Regs(c) => c.clone(),
            SysvPassing::X87 | SysvPassing::X87Pair | SysvPassing::Memory => {
                vec![EightbyteClass::Memory]
            }
        }
    }

    /// SSE + SSEUP: the whole 16-byte value in ONE `%xmm` register.
    pub fn one_xmm(&self) -> bool {
        self.vector_reg_bytes() == Some(16)
    }

    /// The whole value in ONE vector register: `Some(16)` for SSE + SSEUP
    /// (`%xmm`), `Some(32)` for SSE + 3×SSEUP (`%ymm`, AVX), `Some(64)` for
    /// SSE + 7×SSEUP (`%zmm`, AVX-512); `None` for everything else.
    pub fn vector_reg_bytes(&self) -> Option<usize> {
        match self {
            SysvPassing::Regs(c) => vector_reg_bytes_of(c),
            _ => None,
        }
    }
}

/// `Some(bytes)` when `classes` is the one-vector-register shape SSE
/// followed only by SSEUPs with 2, 4 or 8 entries (16, 32 or 64 bytes).
pub fn vector_reg_bytes_of(classes: &[EightbyteClass]) -> Option<usize> {
    match classes {
        [EightbyteClass::Sse, rest @ ..]
            if matches!(rest.len(), 1 | 3 | 7)
                && rest.iter().all(|c| *c == EightbyteClass::SseUp) =>
        {
            Some((rest.len() + 1) * 8)
        }
        _ => None,
    }
}

/// The most eightbytes an object may span and still be classified: two,
/// or the AVX / AVX-512 vector-register width when the TU's ABI level
/// grants it (`target_vector_abi_bytes`).
fn max_words() -> usize {
    super::types::target_vector_abi_bytes() / 8
}

/// psABI post-merger rule (c): an object of more than two eightbytes is
/// MEMORY unless its first eightbyte is SSE and every other one is SSEUP
/// (the one-`%ymm` / one-`%zmm` shape). GCC applies this at every
/// aggregate level (`classify_argument`), and so does `classify_at`.
fn wide_shape_ok(classes: &[Class]) -> bool {
    classes.len() <= 2
        || (classes[0] == Class::Sse && classes[1..].iter().all(|c| *c == Class::SseUp))
}

/// Classify `ty` for by-value passing. `None` for types that are not
/// aggregates (scalars have their own conventions) and for incomplete types.
pub fn classify(ty: &CType, ctx: &dyn StructLayoutProvider) -> Option<SysvPassing> {
    if !matches!(ty, CType::Struct(_) | CType::Union(_)) && !ty.is_vector() {
        return None;
    }
    // GCC 16.2.0 (measured, harness-cache-1 oracle): a one-lane FP
    // `vector_size` vector has no vector mode (BLKmode) and is MEMORY in the
    // argument position — the 4/8 bytes travel on the stack
    // (`f32x1 id(f32x1)` loads its argument from `8(%rsp)`). Everything the
    // `None` leaf below used to fall back to (an INTEGER register pass)
    // diverged from the oracle by a whole calling convention.
    if let CType::Vector(elem, total) = ty {
        if matches!(**elem, CType::Float | CType::Double) && elem.size() == *total {
            return Some(SysvPassing::Memory);
        }
    }
    if let CType::Struct(key) | CType::Union(key) = ty {
        ctx.get_struct_layout(key)?;
    }
    Some(finish(classify_at(ty, 0, ctx)))
}

/// The return-position convention: `classify`, except for the one place
/// Clang's return classification departs from its argument classification.
/// A bare one-lane `double` ext vector is MEMORY as an argument ("gcc passes
/// <1 x double> in memory", Clang's X86_64ABIInfo) yet comes back as the SSE
/// scalar in `%xmm0` (Clang 23.1.3: `f64x1 id(f64x1 v)` is
/// `movsd 8(%rsp), %xmm0; ret`). As a member it is MEMORY in both positions.
pub fn classify_return(ty: &CType, ctx: &dyn StructLayoutProvider) -> Option<SysvPassing> {
    if let CType::ExtVector(elem, layout) = ty {
        if layout.lanes == 1 && matches!(**elem, CType::Double) {
            return Some(SysvPassing::Regs(vec![EightbyteClass::Sse]));
        }
        // The float twin: a one-lane `float` ext vector comes back as the
        // scalar in the low half of %xmm0, exactly like f64x1 (same-compiler
        // GCC 16 / Clang codegen, pinned end to end by
        // `check_ext_vector_one_lane_fp.sh`). Classified INTEGER here, the
        // callee returned the bits in %rax while the indirect-call carrier
        // (and both oracles) say %xmm0 — a cross-TU ABI seam.
        if layout.lanes == 1 && matches!(**elem, CType::Float) {
            return Some(SysvPassing::Regs(vec![EightbyteClass::Sse]));
        }
        // 256 bits is MEMORY without AVX, but a vector is not an aggregate
        // for Clang's ABI code, so the return goes "direct" to LLVM, which
        // scalarizes `<2 x x86_fp80>` into the x87 register pair.
        if layout.lanes == 2 && matches!(**elem, CType::LongDouble) {
            return Some(SysvPassing::X87Pair);
        }
    }
    // The `vector_size` spelling of the one-lane FP shapes is answered by
    // `classify` itself: GCC 16.2.0 measured V1SF/V1DF as MEMORY (sret), so
    // no return-position departure exists there — see the arm in `classify`.
    // The `ext_vector_type` spellings follow Clang (the only oracle that
    // accepts the attribute) and keep their SSE return arms.
    classify(ty, ctx)
}

/// Classify a record by its layout (for callers holding a layout, not a type).
pub fn classify_layout(layout: &StructLayout, ctx: &dyn StructLayoutProvider) -> SysvPassing {
    let words = layout.size.div_ceil(8);
    let classes = match words {
        0 => Some(vec![Class::NoClass]),
        w if w <= max_words() => {
            classify_record(layout, words, 0, ctx).filter(|c| wide_shape_ok(c))
        }
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
                // Defense in depth, not dead code: the X87/X87Up/ComplexX87/
                // Memory arm above only inspects the classes slice directly.
                // If a future edit routes a classified-but-unfiltered type
                // through this mapping (e.g. a union post-merge pass), the
                // psABI-mandated answer for those classes is memory class —
                // a panic would take a working compile down with it.
                X87 | X87Up | ComplexX87 | Memory => EightbyteClass::Memory,
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
    if words > max_words() {
        return None;
    }
    match ty {
        CType::Struct(key) | CType::Union(key) => {
            classify_record(ctx.get_struct_layout(key)?, words, bit_offset, ctx)
                .filter(|c| wide_shape_ok(c))
        }
        CType::Array(elem, _) => {
            let sub = classify_at(elem, bit_offset, ctx)?;
            Some(
                (0..words)
                    .map(|i| sub[i % sub.len()])
                    .collect::<Vec<Class>>(),
            )
            .filter(|c| wide_shape_ok(c))
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
            if lanes_fp && elem.size() == *total {
                return None; // V1SF / V1DF: no vector mode, BLKmode
            }
            match *total {
                // `__m256` / `__m512`: one `%ymm` / `%zmm` under the AVX
                // (AVX-512) ABI level, MEMORY below it (`classify_at`
                // already returned None for the latter: the object spans
                // more eightbytes than `max_words`).
                32 | 64 => (wide_vector_classes(*total), *total * 8),
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

/// SSE followed by SSEUPs over every eightbyte of a `total`-byte vector
/// (32 or 64): the one-`%ymm` / one-`%zmm` carrier.
fn wide_vector_classes(total: usize) -> Vec<Class> {
    let mut classes = vec![Class::SseUp; total / 8];
    classes[0] = Class::Sse;
    classes
}

/// Clang's x86-64 classification of an `ext_vector_type` (measured against
/// Clang 23.1.3, including bit-packed `_Bool` vectors, padded three-lane
/// shapes and members): 1, 2 and 4 bytes are INTEGER; 8 bytes are SSE except
/// one `double` lane (MEMORY); 16 bytes are SSE + SSEUP (except a 128-lane
/// `_Bool` vector, MEMORY); 32 and 64 bytes are SSE + SSEUPs — one `%ymm` /
/// `%zmm` — when the TU's AVX level admits them (`X86_64ABIInfo::classify`:
/// `Size == 128 || (IsNamedArg && Size <= getNativeVectorSizeForAVXABI)`)
/// and MEMORY otherwise. A
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
        1 | 2 | 4 => (vec![Integer], size * 8),
        8 if layout.lanes == 1 && matches!(elem, CType::Double) => return None,
        8 => (vec![Sse], 64),
        16 => (vec![Sse, SseUp], 128),
        32 | 64 if layout.bitpacked_bool => return None,
        32 | 64 => (wide_vector_classes(size), size * 8),
        _ => return None,
    })
}

#[cfg(test)]
mod long_double_vector_tests {
    use super::*;
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::{ExtVectorLayout, RcLayout};

    fn ld_vec(lanes: usize) -> CType {
        let layout = ExtVectorLayout::new(&CType::LongDouble, lanes)
            .expect("x86 long double ext vector layout");
        CType::ExtVector(Box::new(CType::LongDouble), layout)
    }

    /// ABI-EXTVEC-LD-1: the `long double` lane conventions Clang's
    /// `X86_64ABIInfo` produces (and that GCC's x87 struct / `_Complex long
    /// double` conventions share) — one lane is the X87 class, two lanes are
    /// MEMORY as an argument but the `%st(0)/%st(1)` pair on return, three
    /// lanes are MEMORY in both positions (LLVM sret-demotes the "direct"
    /// `<3 x x86_fp80>` return, which is the hidden pointer lccc uses).
    #[test]
    fn long_double_lanes_classify_like_clang() {
        let ctx: FxHashMap<String, RcLayout> = FxHashMap::default();
        assert_eq!(classify(&ld_vec(1), &ctx), Some(SysvPassing::X87));
        assert_eq!(classify_return(&ld_vec(1), &ctx), Some(SysvPassing::X87));

        assert_eq!(classify(&ld_vec(2), &ctx), Some(SysvPassing::Memory));
        assert_eq!(
            classify_return(&ld_vec(2), &ctx),
            Some(SysvPassing::X87Pair)
        );
        assert_eq!(
            SysvPassing::X87Pair.arg_classes(),
            vec![EightbyteClass::Memory]
        );
        assert!(!SysvPassing::X87Pair.one_xmm());

        assert_eq!(classify(&ld_vec(3), &ctx), Some(SysvPassing::Memory));
        assert_eq!(classify_return(&ld_vec(3), &ctx), Some(SysvPassing::Memory));
    }
}
