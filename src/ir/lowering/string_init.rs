//! String literals used as array initializers (C11 6.7.9p14-15).
//!
//! An array of character type may be initialized by a character string
//! literal (`"..."`, `u8"..."`), an array whose element type is compatible
//! with `wchar_t`, `char16_t` or `char32_t` by the matching wide literal
//! (`L"..."`, `u"..."`, `U"..."`), optionally enclosed in braces.  Successive
//! code units initialize the elements; the terminating null initializes one
//! more element only when the array has room for it, and elements beyond
//! that are zero (p21, as for any partially initialized aggregate).
//!
//! Every initializer path (block-scope stores, static byte images, the
//! per-element constant lists of multi-dimensional statics) recognises the
//! literal through [`StringInit`], so that the width match and the code-unit
//! encoding are decided in exactly one place.  Before this module the
//! recognition was repeated per path and mostly written for `char` only: a
//! wide literal in a multi-dimensional array, a struct member, an array of
//! structs or a compound literal fell through to the scalar path, which
//! stored the literal's ADDRESS into the first elements.

use super::lower::Lowerer;
use crate::common::types::{CType, IrType};
use crate::frontend::parser::ast::{Expr, Initializer, InitializerItem};
use crate::ir::reexports::{IrConst, Value};

/// Encoding of one code unit of a string literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodeUnit {
    /// `"..."` / `u8"..."`: the lexer stores one `char` per C byte
    /// (U+0000..=U+00FF), so the byte is the `char`'s value.
    Byte,
    /// `u"..."`: UTF-16 (C11 6.4.5p6, `__STDC_UTF_16__`); a code point above
    /// U+FFFF is a surrogate pair, i.e. two elements.
    Utf16,
    /// `L"..."` / `U"..."`: one element per code point (`wchar_t` is 32-bit
    /// on every lccc target).
    Utf32,
}

/// A string literal that initializes an array of matching element width.
#[derive(Clone, Copy, Debug)]
pub(super) struct StringInit<'a> {
    text: &'a str,
    unit: CodeUnit,
}

impl<'a> StringInit<'a> {
    /// `e` when it is a string literal whose code-unit width equals the
    /// width of the integer element type `elem`.  Literal kinds are matched
    /// to widths, not to exact C types, because the element type reaches the
    /// initializer paths as an `IrType` (signedness and the `int`/`long`
    /// spelling of a 32-bit `wchar_t` are not visible there, and do not
    /// matter: C11 6.7.9p15 only asks for compatibility with the character
    /// type, which has that width on all targets).
    pub(super) fn for_elem_ir(e: &'a Expr, elem: IrType) -> Option<Self> {
        let (text, unit) = match e {
            Expr::StringLiteral(s, _) => (s.as_str(), CodeUnit::Byte),
            Expr::Char16StringLiteral(s, _) => (s.as_str(), CodeUnit::Utf16),
            Expr::WideStringLiteral(s, _) => (s.as_str(), CodeUnit::Utf32),
            _ => return None,
        };
        let matches = match unit {
            CodeUnit::Byte => matches!(elem, IrType::I8 | IrType::U8),
            CodeUnit::Utf16 => matches!(elem, IrType::I16 | IrType::U16),
            CodeUnit::Utf32 => matches!(elem, IrType::I32 | IrType::U32),
        };
        matches.then_some(Self { text, unit })
    }

    /// As [`Self::for_elem_ir`] for a C element type.  `_Bool` and
    /// enumerations are not character types even where their width agrees.
    pub(super) fn for_elem_ctype(e: &'a Expr, elem: &CType) -> Option<Self> {
        if !elem.is_integer() || matches!(elem, CType::Bool | CType::Enum(_)) {
            return None;
        }
        Self::for_elem_ir(e, IrType::from_ctype(elem))
    }

    /// The literal initializing an array of `elem`: `e` itself, or the sole
    /// undesignated member of a brace list (`char a[4] = { "ab" };`).
    pub(super) fn for_init_ctype(init: &'a Initializer, elem: &CType) -> Option<Self> {
        match init {
            Initializer::Expr(e) => Self::for_elem_ctype(e, elem),
            Initializer::List(items) => {
                Self::sole_item(items).and_then(|e| Self::for_elem_ctype(e, elem))
            }
        }
    }

    /// The single undesignated expression of a brace list, if that is all
    /// the list holds.
    pub(super) fn sole_item(items: &'a [InitializerItem]) -> Option<&'a Expr> {
        match items {
            [item] if item.designators.is_empty() => match &item.init {
                Initializer::Expr(e) => Some(e),
                Initializer::List(_) => None,
            },
            _ => None,
        }
    }

    /// Width in bytes of one element.
    pub(super) fn unit_size(self) -> usize {
        match self.unit {
            CodeUnit::Byte => 1,
            CodeUnit::Utf16 => 2,
            CodeUnit::Utf32 => 4,
        }
    }

    /// The code units, without the terminator.
    pub(super) fn units(self) -> Vec<u32> {
        match self.unit {
            CodeUnit::Byte => self.text.chars().map(|c| c as u32 & 0xff).collect(),
            CodeUnit::Utf16 => self.text.encode_utf16().map(u32::from).collect(),
            CodeUnit::Utf32 => self.text.chars().map(|c| c as u32).collect(),
        }
    }

    /// Elements the literal occupies including its terminator: the length
    /// an unsized array (`T a[] = "..."`) receives.
    pub(super) fn elems_with_nul(self) -> usize {
        let n = match self.unit {
            CodeUnit::Byte | CodeUnit::Utf32 => self.text.chars().count(),
            CodeUnit::Utf16 => self.text.encode_utf16().count(),
        };
        n + 1
    }

    /// Bytes the literal occupies including its terminator.
    pub(super) fn bytes_with_nul(self) -> usize {
        self.elems_with_nul() * self.unit_size()
    }

    /// The elements actually stored into an array of `max_bytes` bytes:
    /// the code units that fit, then the terminator if there is room for it
    /// (p14 drops it when the array holds exactly the characters).
    pub(super) fn stored_units(self, max_bytes: usize) -> Vec<u32> {
        let room = max_bytes / self.unit_size();
        let mut units = self.units();
        units.truncate(room);
        if units.len() < room {
            units.push(0);
        }
        units
    }

    /// Write the stored elements little-endian into a static byte image at
    /// `offset`.  Bytes after them are left as they are: static images start
    /// zeroed, which provides p21's zero tail.
    pub(super) fn write_bytes(self, bytes: &mut [u8], offset: usize, max_bytes: usize) {
        let size = self.unit_size();
        for (i, u) in self.stored_units(max_bytes).into_iter().enumerate() {
            let at = offset + i * size;
            let le = u.to_le_bytes();
            for (k, &b) in le[..size].iter().enumerate() {
                if let Some(slot) = bytes.get_mut(at + k) {
                    *slot = b;
                }
            }
        }
    }

    /// The elements of an array of `elems` elements of type `ty` initialized
    /// by the literal, zero-padded to `elems`, as constants for the
    /// per-element lists of multi-dimensional statics.
    pub(super) fn element_consts(self, elems: usize, ty: IrType) -> impl Iterator<Item = IrConst> {
        let units = self.stored_units(elems * self.unit_size());
        let pad = elems.saturating_sub(units.len());
        units
            .into_iter()
            .chain(std::iter::repeat_n(0, pad))
            .map(move |u| IrConst::from_i64(u as i64, ty))
    }
}

impl Lowerer {
    /// Store the literal into a local object at `offset`, bounded by the
    /// array's `max_bytes`.  Returns the byte length of the literal with its
    /// terminator, so a caller whose object is not zero-filled beforehand can
    /// clear the tail with [`Self::zero_fill_after_string`].
    pub(super) fn emit_string_init_to_alloca(
        &mut self,
        alloca: Value,
        init: StringInit<'_>,
        offset: usize,
        max_bytes: usize,
    ) -> usize {
        match init.unit {
            CodeUnit::Byte => self.emit_string_to_alloca(alloca, init.text, offset, max_bytes),
            CodeUnit::Utf16 => {
                self.emit_char16_string_to_alloca(alloca, init.text, offset, max_bytes)
            }
            CodeUnit::Utf32 => {
                self.emit_wide_string_to_alloca(alloca, init.text, offset, max_bytes)
            }
        }
        init.bytes_with_nul()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::source::Span;

    fn lit(kind: u8, s: &str) -> Expr {
        let s = s.to_string();
        match kind {
            b'n' => Expr::StringLiteral(s, Span::dummy()),
            b'u' => Expr::Char16StringLiteral(s, Span::dummy()),
            _ => Expr::WideStringLiteral(s, Span::dummy()),
        }
    }

    #[test]
    fn widths_must_match_the_element() {
        let n = lit(b'n', "ab");
        let u = lit(b'u', "ab");
        let w = lit(b'L', "ab");
        assert!(StringInit::for_elem_ir(&n, IrType::I8).is_some());
        assert!(StringInit::for_elem_ir(&n, IrType::U8).is_some());
        assert!(StringInit::for_elem_ir(&n, IrType::I16).is_none());
        assert!(StringInit::for_elem_ir(&u, IrType::U16).is_some());
        assert!(StringInit::for_elem_ir(&u, IrType::I32).is_none());
        assert!(StringInit::for_elem_ir(&w, IrType::I32).is_some());
        assert!(StringInit::for_elem_ir(&w, IrType::U32).is_some());
        assert!(StringInit::for_elem_ir(&w, IrType::I64).is_none());
        assert!(StringInit::for_elem_ctype(&n, &CType::Bool).is_none());
        assert!(StringInit::for_elem_ctype(&w, &CType::Float).is_none());
        assert!(StringInit::for_elem_ctype(&u, &CType::UShort).is_some());
    }

    #[test]
    fn terminator_only_when_there_is_room() {
        let w = lit(b'L', "ab");
        let s = StringInit::for_elem_ir(&w, IrType::I32).unwrap();
        assert_eq!(s.stored_units(16), vec![0x61, 0x62, 0]);
        assert_eq!(s.stored_units(12), vec![0x61, 0x62, 0]);
        assert_eq!(s.stored_units(8), vec![0x61, 0x62]);
        assert_eq!(s.stored_units(4), vec![0x61]);
        assert_eq!(s.stored_units(3), Vec::<u32>::new());
    }

    #[test]
    fn utf16_uses_surrogate_pairs() {
        let u = lit(b'u', "a\u{1F600}");
        let s = StringInit::for_elem_ir(&u, IrType::U16).unwrap();
        assert_eq!(s.units(), vec![0x61, 0xD83D, 0xDE00]);
        assert_eq!(s.elems_with_nul(), 4);
        assert_eq!(s.bytes_with_nul(), 8);
        // Exact fit keeps the high surrogate and drops the terminator.
        assert_eq!(s.stored_units(4), vec![0x61, 0xD83D]);
        let mut img = [0xAAu8; 10];
        s.write_bytes(&mut img, 1, 8);
        assert_eq!(img, [0xAA, 0x61, 0, 0x3D, 0xD8, 0x00, 0xDE, 0, 0, 0xAA]);
    }

    #[test]
    fn narrow_bytes_and_padding() {
        let n = lit(b'n', "a\u{e9}");
        let s = StringInit::for_elem_ir(&n, IrType::I8).unwrap();
        assert_eq!(s.units(), vec![0x61, 0xE9]);
        let v: Vec<IrConst> = s.element_consts(5, IrType::I8).collect();
        assert_eq!(v.len(), 5);
        assert_eq!(v[1], IrConst::from_i64(0xE9, IrType::I8));
        assert_eq!(v[4], IrConst::from_i64(0, IrType::I8));
        let braced = [InitializerItem {
            designators: vec![],
            init: Initializer::Expr(n.clone()),
        }];
        assert!(StringInit::sole_item(&braced).is_some());
    }
}
