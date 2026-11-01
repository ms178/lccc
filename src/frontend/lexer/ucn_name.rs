//! C23 `\N{...}` named universal character escapes: name -> code point.
//!
//! `\N{NAME}` is a *named* universal character name: a `\uXXXX` whose operand
//! is a Unicode character name (`\N{LATIN SMALL LETTER E WITH ACUTE}`) instead
//! of hex digits. It is C++23's spelling, which GCC 16.2, Clang 23.1.4 and lccc
//! also accept in C as an extension (GCC warns about it only under `-pedantic`:
//! "named universal character escapes are only valid in C++23"). Because it is
//! the same grammar production, every rule that applies to `\uXXXX` -- UTF-8 in
//! a narrow literal, the multi-character reading in a character constant, and
//! the rejection inside an identifier -- applies to it unchanged. Only the
//! lookup lives here.
//!
//! The table in [`super::ucn_name_table`] is the Unicode 18.0.0 database with
//! the two algorithmic families folded out; this module is the decoder:
//!
//!   * [`hex_family`] renders the name back out of the candidate code point
//!     and compares, so `CJK UNIFIED IDEOGRAPH-4E00` matches but the same
//!     digits with a leading zero, in lower case, or with an unassigned code
//!     point do not;
//!   * [`hangul`] composes `HANGUL SYLLABLE <L><V><T>` from the jamo names;
//!   * [`explicit`] binary-searches the block index and then decodes at most
//!     [`BLOCK`] front-coded entries.
//!
//! The oracle for every name here is Unicode 18.0.0 (`unicodedata2`), not
//! another compiler: GCC 16.2's table stops one Unicode release short, so
//! `\N{ALCHEMICAL SYMBOL FOR MOON-JUPITER}` (added in Unicode 18) is a valid
//! character name that GCC 16.2 rejects while Clang 23.1.4 accepts it. LCCC
//! follows the Unicode data it documents. `tests/regression/check_ucn_named_escapes.sh`
//! measures both the agreement on the names GCC knows and that deviation.

use super::ucn_name_table::{
    BLOCK, BLOCK_FIRST_NAME, CODEPOINTS, HANGUL_L, HANGUL_T, HANGUL_V, HEX_FAMILIES, MAX_NAME_LEN,
    NAMES_BLOB,
};

/// The hex-suffix families are spelled with four or five uppercase hex digits
/// (`CJK UNIFIED IDEOGRAPH-4E00`, `EGYPTIAN HIEROGLYPH-13460`). Measured on
/// GCC 16.2: it accepts exactly that width and pads a four-digit name to five
/// with a leading zero (`...-04E00` is U+4E00), while `...-004E00` is rejected.
/// The generator asserts that no family in the table needs a wider spelling, so
/// a future Unicode that introduces one fails the generator instead of silently
/// disagreeing with the oracle.
const HEX_MIN_DIGITS: usize = 4;
const HEX_MAX_DIGITS: usize = 5;

/// `SBase` of the Unicode Hangul syllable arithmetic (UAX #44, `Hangul.txt`);
/// syllable `s` is `S_BASE + s`, and `s = (LIndex * V_COUNT + VIndex) * T_COUNT
/// + TIndex`.
const HANGUL_S_BASE: u32 = 0xAC00;
const HANGUL_V_COUNT: u32 = 21;
const HANGUL_T_COUNT: u32 = 28;

/// Resolve a named universal character escape. `name` is the byte string
/// between the braces, which the lexer has already delimited.
pub(crate) fn lookup(name: &[u8]) -> Option<u32> {
    if name.is_empty() {
        return None;
    }
    hex_family(name)
        .or_else(|| hangul(name))
        .or_else(|| explicit(name))
}

/// `<PREFIX><UPPERCASE HEX>` names, e.g. `CJK UNIFIED IDEOGRAPH-4E00`.
fn hex_family(name: &[u8]) -> Option<u32> {
    for (prefix, ranges) in HEX_FAMILIES {
        let Some(digits) = name.strip_prefix(prefix.as_bytes()) else {
            continue;
        };
        // Bounded before parsing, so an adversarial `\N{...}` cannot overflow.
        if digits.len() < HEX_MIN_DIGITS || digits.len() > HEX_MAX_DIGITS {
            return None;
        }
        let mut value: u32 = 0;
        for &digit in digits {
            // Uppercase only: the Unicode data spells these names in upper
            // case, and GCC rejects `\N{CJK UNIFIED IDEOGRAPH-4e00}`.
            let hex = match digit {
                b'0'..=b'9' => digit - b'0',
                b'A'..=b'F' => digit - b'A' + 10,
                _ => return None,
            };
            value = value * 16 + u32::from(hex);
        }
        // Render the name back out of the code point and compare, ignoring the
        // leading zero of a four-digit name padded to five: this is what makes
        // the fold exact rather than approximate, so a lower-case digit, a
        // wrong digit count or a value outside the family's assigned ranges
        // cannot alias a real name.
        let significant = match digits.iter().position(|&b| b != b'0') {
            Some(start) => &digits[start..],
            None => return None,
        };
        let mut rendered = [0u8; 6];
        if render_upper_hex(value, &mut rendered) != significant {
            return None;
        }
        let assigned = ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&value));
        return assigned.then_some(value);
    }
    None
}

/// `HANGUL SYLLABLE <L><V><T>` from the 19/21/27 jamo names.
fn hangul(name: &[u8]) -> Option<u32> {
    let rest = name.strip_prefix("HANGUL SYLLABLE ".as_bytes())?;
    let mut found: Option<u32> = None;
    for (l_index, l_name) in HANGUL_L.iter().enumerate() {
        let Some(after_l) = rest.strip_prefix(l_name.as_bytes()) else {
            continue;
        };
        for (v_index, v_name) in HANGUL_V.iter().enumerate() {
            let Some(after_v) = after_l.strip_prefix(v_name.as_bytes()) else {
                continue;
            };
            for (t_index, t_name) in HANGUL_T.iter().enumerate() {
                if after_v != t_name.as_bytes() {
                    continue;
                }
                let index = (l_index as u32 * HANGUL_V_COUNT + v_index as u32) * HANGUL_T_COUNT
                    + t_index as u32;
                let code_point = HANGUL_S_BASE + index;
                // The composition is injective (the generator asserts it), so a
                // second match would mean a corrupt table; refuse to guess.
                if found.replace(code_point).is_some() {
                    return None;
                }
            }
        }
    }
    found
}

/// The front-coded explicit names.
fn explicit(name: &[u8]) -> Option<u32> {
    if name.len() > MAX_NAME_LEN {
        return None;
    }
    let mut low = 0usize;
    let mut high = BLOCK_FIRST_NAME.len();
    while low < high {
        let middle = low + (high - low) / 2;
        let first = full_name_at(BLOCK_FIRST_NAME[middle] as usize)?;
        match first.cmp(name) {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => return Some(CODEPOINTS[middle * BLOCK]),
        }
    }
    // `low` is the first block whose first name sorts after `name`, so the only
    // block that can hold it is the one before.
    let block = low.checked_sub(1)?;
    let first_entry = block * BLOCK;
    let end = (first_entry + BLOCK).min(CODEPOINTS.len());
    let mut offset = BLOCK_FIRST_NAME[block] as usize;
    let mut decoded = [0u8; MAX_NAME_LEN];
    let mut length = 0usize;
    for index in first_entry..end {
        let prefix_length = usize::from(*NAMES_BLOB.get(offset)?);
        let suffix_start = offset + 1;
        let suffix_length = NAMES_BLOB
            .get(suffix_start..)?
            .iter()
            .position(|&b| b == 0)?;
        let suffix = &NAMES_BLOB[suffix_start..suffix_start + suffix_length];
        offset = suffix_start + suffix_length + 1;
        // The decoded name is the first `prefix_length` bytes of the previous
        // name plus this entry's suffix, so *its* length is what must fit --
        // computing the bound from the previous name's length instead rejects
        // every name longer than its predecessor (`... MAKSURA FINAL FORM`
        // follows `... THEH WITH ZAIN FINAL FORM`).
        if prefix_length > length {
            return None; // corrupt table: the prefix exceeds the previous name
        }
        let name_length = prefix_length + suffix_length;
        if name_length > MAX_NAME_LEN {
            return None;
        }
        // `decoded[..prefix_length]` already holds the shared prefix from the
        // previous entry; only the suffix has to be written.
        decoded[prefix_length..name_length].copy_from_slice(suffix);
        length = name_length;
        if decoded[..length] == *name {
            return Some(CODEPOINTS[index]);
        }
    }
    None
}

/// The full (unfront-coded) name of a block's first entry.
fn full_name_at(offset: usize) -> Option<&'static [u8]> {
    // A block always restarts the prefix chain, so its first byte is 0.
    let prefix_length = *NAMES_BLOB.get(offset)?;
    if prefix_length != 0 {
        return None;
    }
    let start = offset + 1;
    let length = NAMES_BLOB.get(start..)?.iter().position(|&b| b == 0)?;
    Some(&NAMES_BLOB[start..start + length])
}

/// `value` as uppercase hex with no leading zeros, written into `buffer`
/// (which must hold six digits) and returned as a slice; `value == 0` renders
/// as `0`.
fn render_upper_hex(value: u32, buffer: &mut [u8; 6]) -> &[u8] {
    let mut digits = 0usize;
    let mut remaining = value;
    loop {
        digits += 1;
        if remaining < 16 {
            break;
        }
        remaining /= 16;
    }
    let mut cursor = digits;
    let mut remaining = value;
    while cursor > 0 {
        cursor -= 1;
        let digit = (remaining % 16) as u8;
        buffer[cursor] = if digit < 10 {
            b'0' + digit
        } else {
            b'A' + digit - 10
        };
        remaining /= 16;
    }
    &buffer[..digits]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(name: &str) -> Option<u32> {
        lookup(name.as_bytes())
    }

    #[test]
    fn explicit_names() {
        // Ordinary names, including the shortest and the longest in the table.
        assert_eq!(cp("LATIN SMALL LETTER E WITH ACUTE"), Some(0x00E9));
        assert_eq!(cp("LATIN CAPITAL LETTER A"), Some(0x0041));
        assert_eq!(cp("DIGIT ONE"), Some(0x0031));
        assert_eq!(cp("SNOWMAN"), Some(0x2603));
        assert_eq!(cp("SPACE"), Some(0x0020));
        assert_eq!(cp("NO-BREAK SPACE"), Some(0x00A0));
        assert_eq!(cp("ZERO WIDTH JOINER"), Some(0x200D));
        assert_eq!(cp("GRINNING FACE"), Some(0x1F600));
        // A name whose code point is far from any algorithmic family.
        assert_eq!(cp("MUSICAL SYMBOL G CLEF"), Some(0x1D11E));
        assert_eq!(cp("CUNEIFORM SIGN A"), Some(0x12000));
        assert_eq!(cp("ADLAM CAPITAL LETTER ALIF"), Some(0x1E900));
    }

    #[test]
    fn hex_families() {
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-4E00"), Some(0x4E00));
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-20000"), Some(0x20000));
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-31350"), Some(0x31350));
        assert_eq!(cp("CJK COMPATIBILITY IDEOGRAPH-F900"), Some(0xF900));
        assert_eq!(cp("CJK COMPATIBILITY IDEOGRAPH-2F800"), Some(0x2F800));
        assert_eq!(cp("TANGUT IDEOGRAPH-17000"), Some(0x17000));
        assert_eq!(cp("TANGUT IDEOGRAPH-18D20"), Some(0x18D20));
        assert_eq!(cp("KHITAN SMALL SCRIPT CHARACTER-18B00"), Some(0x18B00));
        assert_eq!(cp("NUSHU CHARACTER-1B170"), Some(0x1B170));
        assert_eq!(cp("EGYPTIAN HIEROGLYPH-13460"), Some(0x13460));
        // Unicode 15.1/16 additions: GCC 16.2 knows these two as well.
        assert_eq!(cp("JURCHEN CHARACTER-18E00"), Some(0x18E00));
        assert_eq!(cp("SMALL SEAL CHARACTER-3D000"), Some(0x3D000));
        // A family name must be exact: no leading zeros, no lower case, no
        // unassigned code point inside the family's ranges.
        // A four-digit name may be padded to five with one leading zero, which
        // is what GCC 16.2 accepts; six digits never are.
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-04E00"), Some(0x4E00));
        assert_eq!(cp("CJK COMPATIBILITY IDEOGRAPH-0F900"), Some(0xF900));
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-004E00"), None);
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-4e00"), None);
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-FFFF"), None);
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-50000"), None);
        assert_eq!(cp("CJK UNIFIED IDEOGRAPH-"), None);
        assert_eq!(cp("TANGUT IDEOGRAPH-1A000"), None);
    }

    #[test]
    fn hangul_syllables() {
        assert_eq!(cp("HANGUL SYLLABLE GA"), Some(0xAC00));
        assert_eq!(cp("HANGUL SYLLABLE GAG"), Some(0xAC01));
        // ieung (LIndex 11) has no leading name of its own.
        assert_eq!(cp("HANGUL SYLLABLE A"), Some(0xAC00 + 11 * 588));
        assert_eq!(cp("HANGUL SYLLABLE HIH"), Some(0xD7A3)); // the last syllable
        // G + YEO + G: VIndex 6, TIndex 1.
        assert_eq!(cp("HANGUL SYLLABLE GYEOG"), Some(0xAC00 + 6 * 28 + 1));
        // Every one of the 11,172 syllables resolves to its own code point, and
        // the composition is order-preserving, so this is an exhaustive check
        // of the three jamo tables.
        let leading = HANGUL_L.len() as u32;
        let vowels = HANGUL_V.len() as u32;
        let trailing = HANGUL_T.len() as u32;
        assert_eq!((leading, vowels, trailing), (19, 21, 28));
        for index in 0..leading * vowels * trailing {
            let name = format!(
                "HANGUL SYLLABLE {}{}{}",
                HANGUL_L[(index / (vowels * trailing)) as usize],
                HANGUL_V[((index / trailing) % vowels) as usize],
                HANGUL_T[(index % trailing) as usize]
            );
            assert_eq!(cp(&name), Some(0xAC00 + index), "{name}");
        }
        assert_eq!(cp("HANGUL SYLLABLE XX"), None);
        assert_eq!(cp("HANGUL SYLLABLE"), None);
    }

    #[test]
    fn rejects_what_is_not_a_name() {
        for name in [
            "",
            " ",
            "NOT A REAL NAME",
            "latn small letter e with acute",
            "LATIN SMALL LETTER E WITH ACUTE ",
            " LATIN SMALL LETTER E WITH ACUTE",
            "LATIN  SMALL LETTER E WITH ACUTE",
            "LATIN SMALL LETTER E WITH ACUTE!",
        ] {
            assert_eq!(cp(name), None, "{name:?}");
        }
    }

    #[test]
    fn render_upper_hex_matches_format() {
        let mut buffer = [0u8; 6];
        for value in [
            0u32, 1, 9, 10, 15, 16, 0x4E00, 0x2F800, 0x33479, 0xFFFFF, 0xFFFFFF,
        ] {
            assert_eq!(
                render_upper_hex(value, &mut buffer),
                format!("{value:X}").as_bytes(),
                "{value:X}"
            );
        }
    }

    /// Every name in the front-coded table resolves back to its own code
    /// point. This is the end-to-end check of the decoder: it walks all 35,351
    /// entries rather than a sample, so a wrong block index, a wrong prefix
    /// reconstruction or a wrong buffer bound cannot hide behind a handful of
    /// hand-picked names.
    #[test]
    fn every_explicit_name_round_trips() {
        let mut offset = 0usize;
        let mut previous: Vec<u8> = Vec::new();
        for &expected in CODEPOINTS.iter() {
            let prefix_length = usize::from(NAMES_BLOB[offset]);
            offset += 1;
            let suffix_length = NAMES_BLOB[offset..].iter().position(|&b| b == 0).unwrap();
            let suffix = &NAMES_BLOB[offset..offset + suffix_length];
            offset += suffix_length + 1;
            let mut name = previous[..prefix_length].to_vec();
            name.extend_from_slice(suffix);
            assert_eq!(
                lookup(&name),
                Some(expected),
                "{} ({:?})",
                String::from_utf8_lossy(&name),
                name.len()
            );
            previous = name;
        }
    }

    /// Every code point named by an algorithmic family resolves back to
    /// itself, name by name: the whole `<PREFIX><HEX>` set and all Hangul
    /// syllables were covered above, so this is the range side of it.
    #[test]
    fn every_family_member_round_trips() {
        for (prefix, ranges) in HEX_FAMILIES {
            for &(low, high) in *ranges {
                for value in low..=high {
                    let name = format!("{prefix}{value:X}");
                    assert_eq!(lookup(name.as_bytes()), Some(value), "{name}");
                }
            }
        }
    }

    /// The table must not contain a name twice: the binary search and the
    /// Hangul composition both assume uniqueness. This walks the blob once.
    #[test]
    fn table_is_sorted_and_unique() {
        let mut previous: Option<Vec<u8>> = None;
        let mut offset = 0usize;
        for index in 0..CODEPOINTS.len() {
            let prefix_length = usize::from(NAMES_BLOB[offset]);
            offset += 1;
            let length = NAMES_BLOB[offset..].iter().position(|&b| b == 0).unwrap();
            let suffix = &NAMES_BLOB[offset..offset + length];
            offset += length + 1;
            let mut name = Vec::new();
            if let Some(previous) = &previous {
                name.extend_from_slice(&previous[..prefix_length]);
            } else {
                assert_eq!(prefix_length, 0, "the first name must be a block start");
            }
            name.extend_from_slice(suffix);
            if let Some(previous) = &previous {
                assert!(name > *previous, "names out of order at {index}");
            }
            if index % BLOCK == 0 {
                assert_eq!(
                    BLOCK_FIRST_NAME[index / BLOCK] as usize,
                    offset - length - 2,
                    "block {index} does not point at its first name"
                );
            }
            previous = Some(name);
        }
        assert_eq!(offset, NAMES_BLOB.len());
    }
}
