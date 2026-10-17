//! Byte images of global objects with a constant, relocation-free initializer.
//!
//! A read of one of their elements or members is a constant in C
//! (C11 6.7.9p4 for an initializer, and GCC folds it: `static const int
//! t[2] = {1, 2}; static int v = t[1];` compiles), so the lowerer must be able
//! to answer `t[1]`, `s.limit` or `m[1].x` with the value the program will
//! actually load.  The authoritative source for that value is the *emitted
//! initializer*: this module turns it back into a byte image -- exactly the
//! bytes the object starts with -- and `const_eval.rs` reads a scalar out of
//! that image at the offset the layout gives.
//!
//! Only relocation-free initializers are kept.  A global holding an address
//! (`static const int *const p = &g;`) has a hole in its image that only the
//! linker can fill, and folding a read of it would invent a value.
//!
//! Nothing here decides *whether* a read is constant: sema does, and rejects
//! the program otherwise (see `static_init.rs::folded_object_read`).  A
//! `None` from this module is a missed fold, never a wrong one, and the
//! measured matrix in `tests/regression/file_scope_init/cases.tsv` (run by
//! `tests/regression/check_file_scope_init.sh`) pins the verdicts against
//! GCC 16.2.

use crate::ir::constants::IrConst;
use crate::ir::module::GlobalInit;

/// A global object's constant byte image, with the type its layout came from.
pub(super) struct ConstObjectImage {
    /// The object's C type (array dimensions and struct identity included):
    /// the offset of an element or member is computed from this.
    pub ctype: crate::common::types::CType,
    /// The bytes the object is initialized with, little-endian.
    pub bytes: Vec<u8>,
}

/// The byte image of `init`, or `None` when it carries a relocation, an
/// aggregate whose leaves are not scalars, or a width this module cannot
/// encode.
///
/// Elements an initializer leaves out are zero (C11 6.7.9p10), and the object
/// is exactly `size` bytes, so a shorter encoding is zero-filled -- the same
/// bytes the backend emits.  A *longer* one means this module's encoding does
/// not describe the initializer: decline rather than hand out an image whose
/// offsets are wrong.
pub(super) fn const_object_bytes(init: &GlobalInit, size: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(size);
    match init {
        GlobalInit::Zero => {
            bytes.resize(size, 0);
            return Some(bytes);
        }
        // A string carries neither its terminator nor the object's tail
        // (`char c[3] = "abc"` has no room for one, `char c[10] = "ab"` has
        // eight zero bytes after it).
        GlobalInit::String(text) => {
            bytes.extend_from_slice(text.as_bytes());
            if bytes.len() < size {
                bytes.push(0);
            }
            return (bytes.len() <= size).then(|| {
                bytes.resize(size, 0);
                bytes
            });
        }
        GlobalInit::WideString(values) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        GlobalInit::Char16String(values) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        _ => {
            if !append_bytes(init, &mut bytes) {
                return None;
            }
        }
    }
    (bytes.len() <= size).then(|| {
        bytes.resize(size, 0);
        bytes
    })
}

fn append_bytes(init: &GlobalInit, out: &mut Vec<u8>) -> bool {
    match init {
        GlobalInit::Zero => false,
        GlobalInit::Scalar(value) => append_scalar(value, out),
        GlobalInit::Array(values) => {
            // `GlobalInit::Array` carries one entry per element (scalars), or
            // one `I8` per byte for a struct image.  A `Zero` entry has no
            // width of its own, so an array holding one is not encoded here.
            values.iter().all(|v| append_scalar(v, out))
        }
        GlobalInit::String(text) => {
            out.extend_from_slice(text.as_bytes());
            out.push(0);
            true
        }
        GlobalInit::WideString(values) => {
            for value in values {
                out.extend_from_slice(&value.to_le_bytes());
            }
            out.extend_from_slice(&0u32.to_le_bytes());
            true
        }
        GlobalInit::Char16String(values) => {
            for value in values {
                out.extend_from_slice(&value.to_le_bytes());
            }
            out.extend_from_slice(&0u16.to_le_bytes());
            true
        }
        // Addresses and label differences need the linker; `Compound` may hold
        // any of them.
        GlobalInit::GlobalAddr(_) | GlobalInit::GlobalAddrOffset(..) => false,
        GlobalInit::GlobalLabelDiff(..) => false,
        GlobalInit::Compound(items) => items.iter().all(|item| append_bytes(item, out)),
    }
}

/// Append one scalar constant, little-endian, at its natural width.  `Zero`
/// carries no width: the caller decides (see `append_bytes`).
fn append_scalar(value: &IrConst, out: &mut Vec<u8>) -> bool {
    match *value {
        IrConst::I8(v) => out.push(v as u8),
        IrConst::I16(v) => out.extend_from_slice(&v.to_le_bytes()),
        IrConst::I32(v) => out.extend_from_slice(&v.to_le_bytes()),
        IrConst::I64(v) => out.extend_from_slice(&v.to_le_bytes()),
        IrConst::I128(v) => out.extend_from_slice(&(v as u128).to_le_bytes()),
        IrConst::F32(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
        IrConst::F64(v) => out.extend_from_slice(&v.to_bits().to_le_bytes()),
        // A decimal float or a long double is stored by the backend in a
        // format this module does not model.
        IrConst::D32(_) | IrConst::D64(_) | IrConst::LongDouble(..) => return false,
        // `Zero` carries no width of its own; the caller decides.
        IrConst::Zero => return false,
    }
    true
}

/// Read a little-endian scalar of `width` bytes at `offset`.
pub(super) fn read_le(bytes: &[u8], offset: usize, width: usize) -> Option<u64> {
    if width > 8 {
        return None;
    }
    let slice = bytes.get(offset..offset.checked_add(width)?)?;
    let mut buf = [0u8; 8];
    buf[..width].copy_from_slice(slice);
    Some(u64::from_le_bytes(buf))
}

/// The value of a bitfield: `bit_width` bits at `bit_offset` inside the
/// storage unit `raw`, sign-extended when the declared type is signed.
///
/// A 4-bit `int` holding all ones is -1, and an `unsigned a : 5` holding the
/// same bits is 31 -- the sign of the *field*, not of the storage unit.
pub(super) fn extract_bitfield(
    raw: u64,
    bit_offset: u32,
    bit_width: u32,
    signed: bool,
) -> Option<IrConst> {
    if bit_width == 0 || bit_width > 64 || bit_offset >= 64 || bit_offset + bit_width > 64 {
        return None;
    }
    let mask = (1u64 << bit_width) - 1;
    let field = (raw >> bit_offset) & mask;
    let value = if signed && bit_width < 64 {
        let shift = 64 - bit_width;
        ((field << shift) as i64) >> shift
    } else {
        field as i64
    };
    Some(match bit_width {
        0..=8 => IrConst::I8(value as i8),
        9..=16 => IrConst::I16(value as i16),
        17..=32 => IrConst::I32(value as i32),
        _ => IrConst::I64(value),
    })
}
