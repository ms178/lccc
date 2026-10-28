//! Which code points a universal character name may denote in an identifier.
//!
//! A raw UTF-8 byte and a `\uXXXX` escape are judged by the *same* set, and
//! the set is neither "alphabetic" nor Unicode `XID_Start`/`XID_Continue`:
//! measured against GCC 16.2, `€` (U+20AC, Sc), U+200B (Cf), U+0301 (Mn) and
//! emoji are identifier characters, while U+00A9 (So), U+E000 and U+F0000
//! (private use) are not, and U+0301 is legal only *inside* an identifier.
//! Those are exactly the Annex D tables of C99/C11 ("Ranges of characters
//! allowed in identifiers"), which is what a compiler must implement here.
//!
//! PROVENANCE -- these tables are not transcribed by hand and not inferred
//! from a Unicode property table. They were measured by asking the oracle:
//! every code point in `0xA0..=0x10FFFF` (minus the surrogate block) was
//! placed both as the first and as the second character of an identifier in a
//! generated translation unit, and `gcc-16 -E` was asked to accept or reject
//! each one. Strictly increasing ranges of accepted code points are recorded
//! verbatim. `tests/regression/check_ucn_identifier_table.sh` re-runs a sample
//! of that measurement against the oracle on every CI run, so the table cannot
//! silently drift from the compiler it claims parity with.
//!
//! The two tables are ordered and disjoint, which the accessors rely on.

/// Code points allowed as the first character of an identifier (Annex D.1).
pub(crate) const ID_START: &[(u32, u32)] = &[
    (0xA8, 0xA8),
    (0xAA, 0xAA),
    (0xAD, 0xAD),
    (0xAF, 0xAF),
    (0xB2, 0xB5),
    (0xB7, 0xBA),
    (0xBC, 0xBE),
    (0xC0, 0xD6),
    (0xD8, 0xF6),
    (0xF8, 0x2FF),
    (0x370, 0x167F),
    (0x1681, 0x180D),
    (0x180F, 0x1DBF),
    (0x1E00, 0x1FFF),
    (0x200B, 0x200D),
    (0x202A, 0x202E),
    (0x203F, 0x2040),
    (0x2054, 0x2054),
    (0x2060, 0x20CF),
    (0x2100, 0x218F),
    (0x2460, 0x24FF),
    (0x2776, 0x2793),
    (0x2C00, 0x2DFF),
    (0x2E80, 0x2FFF),
    (0x3004, 0x3007),
    (0x3021, 0x302F),
    (0x3031, 0xD7FF),
    (0xF900, 0xFDCF),
    (0xFDF0, 0xFE1F),
    (0xFE30, 0xFE44),
    (0xFE47, 0xFFFD),
    (0x10000, 0x1FFFD),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
    (0x40000, 0x4FFFD),
    (0x50000, 0x5FFFD),
    (0x60000, 0x6FFFD),
    (0x70000, 0x7FFFD),
    (0x80000, 0x8FFFD),
    (0x90000, 0x9FFFD),
    (0xA0000, 0xAFFFD),
    (0xB0000, 0xBFFFD),
    (0xC0000, 0xCFFFD),
    (0xD0000, 0xDFFFD),
    (0xE0000, 0xEFFFD),
];

/// Code points allowed after the first character (Annex D.1 with the D.2
/// additions: combining marks and connector punctuation).
pub(crate) const ID_CONTINUE: &[(u32, u32)] = &[
    (0xA8, 0xA8),
    (0xAA, 0xAA),
    (0xAD, 0xAD),
    (0xAF, 0xAF),
    (0xB2, 0xB5),
    (0xB7, 0xBA),
    (0xBC, 0xBE),
    (0xC0, 0xD6),
    (0xD8, 0xF6),
    (0xF8, 0x167F),
    (0x1681, 0x180D),
    (0x180F, 0x1FFF),
    (0x200B, 0x200D),
    (0x202A, 0x202E),
    (0x203F, 0x2040),
    (0x2054, 0x2054),
    (0x2060, 0x218F),
    (0x2460, 0x24FF),
    (0x2776, 0x2793),
    (0x2C00, 0x2DFF),
    (0x2E80, 0x2FFF),
    (0x3004, 0x3007),
    (0x3021, 0x302F),
    (0x3031, 0xD7FF),
    (0xF900, 0xFDCF),
    (0xFDF0, 0xFE44),
    (0xFE47, 0xFFFD),
    (0x10000, 0x1FFFD),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
    (0x40000, 0x4FFFD),
    (0x50000, 0x5FFFD),
    (0x60000, 0x6FFFD),
    (0x70000, 0x7FFFD),
    (0x80000, 0x8FFFD),
    (0x90000, 0x9FFFD),
    (0xA0000, 0xAFFFD),
    (0xB0000, 0xBFFFD),
    (0xC0000, 0xCFFFD),
    (0xD0000, 0xDFFFD),
    (0xE0000, 0xEFFFD),
];

/// Binary search over a sorted, disjoint range table.
fn in_table(table: &[(u32, u32)], cp: u32) -> bool {
    // `partition_point` finds the first range that could contain `cp`; a single
    // comparison then decides. Three or four steps for the tables above, on a
    // path that is only taken for non-ASCII source.
    match table
        .partition_point(|&(start, _)| start <= cp)
        .checked_sub(1)
    {
        Some(idx) => cp <= table[idx].1,
        None => false,
    }
}

/// May `cp` start an identifier?
pub(crate) fn is_identifier_start(cp: u32) -> bool {
    in_table(ID_START, cp)
}

/// May `cp` appear anywhere but the first position of an identifier?
pub(crate) fn is_identifier_continue(cp: u32) -> bool {
    in_table(ID_CONTINUE, cp)
}
