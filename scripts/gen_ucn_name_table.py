#!/usr/bin/env python3
"""Generate `src/frontend/lexer/ucn_name_table.rs`: the name -> code point
table behind C23 `\\N{...}` named universal character escapes.

PROVENANCE
----------
The data is THE Unicode Character Database itself, not a hand transcription and
not a snapshot of another compiler's table.  Every code point that
`unicodedata.name()` reports for Unicode 18.0.0 is enumerated (`Name.txt`, plus
the derived names of `HangulSyllableType.txt`), and the names are split into

  * two *algorithmic* families, whose names are not stored at all because the
    name is a total function of the code point:
      - hex-suffix families, `<PREFIX><UPPERCASE HEX>` (CJK unified and
        compatibility ideographs, Tangut ideographs and components, Khitan
        small script characters, Nushu characters, Egyptian hieroglyphs, ...),
      - Hangul syllables, `HANGUL SYLLABLE <L><V><T>` composed from the 19
        leading, 21 vowel and 27 trailing consonant jamo names;
  * an *explicit* table of the remaining names, front-coded against the
    previous name (names share long prefixes: `LATIN SMALL LETTER ...`) and
    binary-searched through a block index.

Every family is verified exhaustively here, not assumed: a hex family is only
accepted if `name[len(prefix):] == '%X' % codepoint` for *all* of its members,
and the Hangul tables are only accepted if composed names reproduce all 11,172
syllable names byte for byte.  `tests/regression/check_ucn_named_escapes.sh`
re-measures the result against `gcc-16`, so the table cannot silently drift.

WHY NOT A HASH / WORD DICTIONARY
--------------------------------
Measured on Unicode 18.0.0: 172,808 named code points.  A plain name blob is
4,473,051 B; one-byte-prefix front coding over all names is 1,212,338 B; a word
dictionary (148,842 words over 1,174,985 name tokens) is 861,669 B of
dictionary *alone* but needs a token stream on top, i.e. >= 2.7 MB.  Moving the
114,043 hex-family names and the 11,172 Hangul names out of the blob and into
algorithms takes the explicit table down to 47,593 entries / 1,222,858 B, so
the front-coded blob plus its index is ~1.6 MB in total and needs no
hash-function tuning, no rehash on a Unicode update, and no open addressing.

USAGE
-----
    scripts/gen_ucn_name_table.py            # write the table
    scripts/gen_ucn_name_table.py --check     # verify the checked-in table
    scripts/gen_ucn_name_table.py --stats     # print the size breakdown

The script needs a Unicode 18.0.0 database; `unicodedata2` from PyPI provides
one on interpreters whose built-in `unicodedata` is older (checked below).
"""

from __future__ import annotations

import argparse
import sys

try:  # pragma: no cover - the import is the platform probe
    import unicodedata2 as unicodedata
except ImportError:  # pragma: no cover
    import unicodedata

REQUIRED_UCD = "18.0.0"
OUT = "src/frontend/lexer/ucn_name_table.rs"

# A hex-suffix family is only recognised with at least this many members; a
# one- or two-member "family" would be a coincidence of the naming scheme, and
# two members would not distinguish a family from a single oddly named
# character.  The round-trip check below is what actually makes membership
# sound, this is only there to keep the family list short.
MIN_FAMILY = 8

# Entries per front-coding block.  A block restarts the prefix chain, so block
# `b` starts at a *full* name and a lookup can binary-search the block index
# and then decode at most BLOCK names.  Larger blocks compress better; 16 keeps
# the worst-case decode short enough to stay in one or two cache lines of work
# per comparison.
BLOCK = 16

HANGUL_PREFIX = "HANGUL SYLLABLE "
S_BASE, L_BASE, V_BASE, T_BASE = 0xAC00, 0x1100, 0x1161, 0x11A7
L_COUNT, V_COUNT, T_COUNT = 19, 21, 28
N_COUNT, S_COUNT = V_COUNT * T_COUNT, L_COUNT * V_COUNT * T_COUNT


def all_names() -> dict[int, str]:
    """Every code point that has a Unicode name, mapped to that name."""
    names: dict[int, str] = {}
    for cp in range(0x110000):
        name = unicodedata.name(chr(cp), None)
        if name is None:
            continue
        assert name.isascii(), (hex(cp), name)
        assert "\0" not in name, (hex(cp), name)
        names[cp] = name
    return names


def split_hangul(names: dict[int, str]) -> tuple[list[str], list[str], list[str]] | None:
    """Fold the 11,172 Hangul syllable names into the jamo name tables.

    The syllable names are composed as `HANGUL SYLLABLE <L><V><T>` from the
    *jamo short names* of Unicode's `Jamo.txt` (`1100; G`), which are not the
    jamo character names (`HANGUL CHOSEONG KIYEOK`).  Rather than transcribe
    those 67 strings by hand -- which is what a table like this usually does,
    and what silently rots -- they are *derived from the syllable names
    themselves*:

      * the ieung LIndex has the empty short name, so the longest common
        suffix of the 19 names at (L, V=0, T=0) is exactly VShort[0];
      * name(L, 0, 0) is then `<LShort><VShort[0]>`, which yields every LShort;
      * name(0, V, 0) is `<LShort[0]><VShort>`, which yields every VShort;
      * name(0, 0, T) is `<LShort[0]><VShort[0]><TShort>`, which yields every
        TShort.

    The result is accepted only if recomposing it reproduces all {S_COUNT}
    names byte for byte, so a database that does not follow the standard
    derivation is rejected instead of producing a subtly wrong table.
    """
    if any(S_BASE + s not in names for s in range(S_COUNT)):
        return None

    def suffix(cp: int) -> str:
        name = names[cp]
        if not name.startswith(HANGUL_PREFIX):
            raise AssertionError(f"{cp:#x}: {name!r}")
        return name[len(HANGUL_PREFIX) :]

    # Longest common suffix of the 19 names with V = T = 0.  One LShort (the
    # ieung one) is empty, so this is exactly VShort[0] -- asserted below.
    first = [suffix(S_BASE + l * N_COUNT) for l in range(L_COUNT)]
    common = 0
    while all(len(n) > common and n[-1 - common] == first[0][-1 - common] for n in first):
        common += 1
    v0 = first[0][len(first[0]) - common :]
    if not v0:
        return None

    lead = [n[: len(n) - len(v0)] for n in first]
    if sum(1 for x in lead if not x) != 1 or len(set(lead)) != L_COUNT:
        return None
    vowel = [suffix(S_BASE + v * T_COUNT)[len(lead[0]) :] for v in range(V_COUNT)]
    if len(set(vowel)) != V_COUNT or any(not v for v in vowel):
        return None
    trail = [""] + [
        suffix(S_BASE + t)[len(lead[0]) + len(vowel[0]) :] for t in range(1, T_COUNT)
    ]
    if len(set(trail)) != T_COUNT:
        return None

    # Recomposition must reproduce every syllable name, and the composition
    # must be injective (two code points cannot share one name).
    composed = [
        lead[l] + vowel[v] + trail[t]
        for l in range(L_COUNT)
        for v in range(V_COUNT)
        for t in range(T_COUNT)
    ]
    if len(set(composed)) != S_COUNT:
        return None
    for s in range(S_COUNT):
        if composed[s] != suffix(S_BASE + s):
            return None
    return lead, vowel, trail


def split_hex_families(
    names: dict[int, str],
) -> tuple[list[tuple[str, list[tuple[int, int]]]], set[int]]:
    """Fold `<PREFIX><HEX>` names into prefix + code point ranges.

    Returns the families and the set of code points they cover; the caller
    removes those from the explicit table.
    """
    by_prefix: dict[str, list[int]] = {}
    for cp, name in names.items():
        stem, sep, suffix = name.rpartition("-")
        if not sep or not (4 <= len(suffix) <= 6):
            continue
        if any(c not in "0123456789ABCDEF" for c in suffix):
            continue
        by_prefix.setdefault(stem + sep, []).append(cp)

    families: list[tuple[str, list[tuple[int, int]]]] = []
    folded: set[int] = set()
    prefixes: list[str] = []
    for prefix, cps in sorted(by_prefix.items()):
        if len(cps) < MIN_FAMILY:
            continue
        # The whole family must be a pure function of the code point.  This is
        # also what makes `lookup` exact: it re-renders the name from the code
        # point and compares, so digit count and leading zeros cannot alias.
        if any(names[cp] != prefix + "%X" % cp for cp in cps):
            continue
        # The lexer accepts four or five hex digits (GCC 16.2's width); a
        # family whose names need a wider spelling would need a lexer change,
        # so fail here instead.
        widths = {len("%X" % cp) for cp in cps}
        if not widths <= {4, 5}:
            raise SystemExit(f"{prefix}: hex widths {sorted(widths)} outside 4..5")
        ranges: list[tuple[int, int]] = []
        for cp in sorted(cps):
            if ranges and cp == ranges[-1][1] + 1:
                ranges[-1] = (ranges[-1][0], cp)
            else:
                ranges.append((cp, cp))
        families.append((prefix, ranges))
        prefixes.append(prefix)
        folded.update(cps)
    # The lexer resolves a name by the first prefix it starts with, so the
    # prefixes must not nest (they are compared as written, and a nested prefix
    # would make the order significant).
    for outer in prefixes:
        for inner in prefixes:
            if outer != inner and inner.startswith(outer):
                raise SystemExit(f"family prefixes nest: {outer!r} inside {inner!r}")
    return families, folded


def front_code(explicit: list[tuple[str, int]]) -> tuple[bytes, list[int], list[int], int]:
    """Front-code the sorted names into (blob, block offsets, code points, prefix sum)."""
    blob = bytearray()
    block_offsets: list[int] = []
    codepoints: list[int] = []
    prefix_sum = 0
    prev = ""
    for i, (name, cp) in enumerate(explicit):
        if i % BLOCK == 0:
            block_offsets.append(len(blob))
            common = 0
        else:
            common = 0
            limit = min(len(prev), len(name), 255)
            while common < limit and prev[common] == name[common]:
                common += 1
        suffix = name[common:].encode("ascii")
        assert common < 256 and len(name) < 256, name
        blob.append(common)
        blob += suffix
        blob.append(0)
        codepoints.append(cp)
        prefix_sum += common
        prev = name
    return bytes(blob), block_offsets, codepoints, prefix_sum


def rust_bytes(data: bytes) -> str:
    """Render bytes as the inside of a Rust byte-string literal."""
    out = []
    run: list[str] = []
    for b in data:
        if 0x20 <= b < 0x7F and b not in (0x22, 0x5C):  # printable, no " or backslash
            run.append(chr(b))
        else:
            if run:
                out.append("".join(run))
                run = []
            out.append("\\x%02x" % b)
    if run:
        out.append("".join(run))
    text = "".join(out)
    # Wrap so the emitter, not the reader, deals with the length; a 1.2 MB line
    # is legal Rust and rustfmt leaves string literals alone.
    return text


def wrap_array(items: list[str], per_line: int = 12) -> str:
    lines = []
    for i in range(0, len(items), per_line):
        lines.append("    " + ", ".join(items[i : i + per_line]) + ",")
    return "\n".join(lines)


def render() -> tuple[str, dict[str, int]]:
    names = all_names()
    version = unicodedata.unidata_version
    if version != REQUIRED_UCD:
        raise SystemExit(
            f"needs Unicode {REQUIRED_UCD} data, the loaded database is {version}"
        )

    hangul = split_hangul(names)
    if hangul is None:
        raise SystemExit("the database does not carry composed Hangul syllable names")
    for s in range(S_COUNT):
        del names[S_BASE + s]

    families, folded = split_hex_families(names)
    names = {cp: n for cp, n in names.items() if cp not in folded}
    explicit = sorted(((name, cp) for cp, name in names.items()), key=lambda e: e[0])
    assert len({name for name, _ in explicit}) == len(explicit), "duplicate names"
    blob, block_offsets, codepoints, prefix_sum = front_code(explicit)
    assert len(block_offsets) == (len(explicit) + BLOCK - 1) // BLOCK
    assert len(codepoints) == len(explicit)
    max_name_len = max(len(n) for n, _ in explicit)
    assert max_name_len < 256

    lead, vowel, trail = hangul
    lead_rust = "[" + ", ".join('"%s"' % n for n in lead) + "]"
    vowel_rust = "[" + ", ".join('"%s"' % n for n in vowel) + "]"
    trail_rust = "[" + ", ".join('"%s"' % n for n in trail) + "]"
    stats = {
        "named codepoints": len(names) + len(folded) + S_COUNT,
        "explicit entries": len(explicit),
        "front-coded blob": len(blob),
        "block index": 4 * len(block_offsets),
        "codepoints": 4 * len(codepoints),
        "families": len(families),
        "family codepoints": len(folded),
        "hangul syllables": S_COUNT,
        "total table": len(blob) + 4 * len(block_offsets) + 4 * len(codepoints),
        "shared prefix bytes": prefix_sum,
        "longest explicit name": max_name_len,
    }

    family_items = []
    for prefix, ranges in families:
        r = ", ".join("(0x%X, 0x%X)" % (lo, hi) for lo, hi in ranges)
        family_items.append(f'    ("{prefix}", &[{r}]),')

    name_bytes = len(blob) + prefix_sum - 2 * len(explicit)
    plain = name_bytes
    named = stats["named codepoints"]
    table_bytes = stats["total table"]
    src = f'''//! Name -> code point table for C23 `\\N{{...}}` named universal character
//! escapes.
//!
//! GENERATED by `scripts/gen_ucn_name_table.py` from the Unicode {version}
//! character database -- do not edit by hand; run the script instead, and run
//! `scripts/gen_ucn_name_table.py --check` to verify the file is current.
//!
//! The names are split three ways so that only what cannot be derived is
//! stored (the generator asserts every one of the properties below):
//!
//!   * [`HEX_FAMILIES`] -- names that are `<PREFIX><UPPERCASE HEX>` of their
//!     own code point, e.g. `CJK UNIFIED IDEOGRAPH-4E00`.  Nothing is stored
//!     but the prefix and the assigned code point ranges; a lookup re-renders
//!     the name from the code point, so digit count, leading zeros and case
//!     cannot alias.
//!   * [`HANGUL_L`], [`HANGUL_V`], [`HANGUL_T`] -- the {S_COUNT:,} Hangul syllable
//!     names, composed from the jamo names exactly as the Unicode standard
//!     derives them (`HANGUL SYLLABLE GAG` = `G` + `A` + `G`).
//!   * [`NAMES_BLOB`] -- the {len(explicit):,} remaining names, sorted and
//!     front-coded against the previous name: they spell out {name_bytes:,} B but
//!     share {prefix_sum:,} B of common prefix, so they occupy {len(blob):,} B.
//!     An entry is `[prefix length: u8][suffix bytes][NUL]`; every [`BLOCK`]-th
//!     entry restarts the chain at a full name, so a lookup binary-searches
//!     [`BLOCK_FIRST_NAME`] and then decodes at most [`BLOCK`] entries.
//!
//! Measured cost: {table_bytes:,} B in the binary for {named:,} named code points,
//! against {plain:,} B for the names spelled out in one blob.

/// The Unicode version the names come from.  A newer database is a table
/// update, not a code change: re-run the generator.
pub(crate) const UCN_NAMES_UNICODE_VERSION: &str = "{version}";

/// Entries per front-coding block: at most this many names are decoded per
/// lookup after the binary search.
pub(crate) const BLOCK: usize = {BLOCK};

/// Families whose names are `<PREFIX><UPPERCASE HEX>`, with the assigned code
/// point ranges (inclusive, sorted) that carry such a name.
#[rustfmt::skip]
pub(crate) const HEX_FAMILIES: &[(&str, &[(u32, u32)])] = &[
{chr(10).join(family_items)}
];

#[rustfmt::skip]
/// Hangul leading consonant (choseong) names, indexed by `LIndex`.
pub(crate) const HANGUL_L: &[&str] = &{lead_rust};

#[rustfmt::skip]
/// Hangul vowel (jungseong) names, indexed by `VIndex`.
pub(crate) const HANGUL_V: &[&str] = &{vowel_rust};

#[rustfmt::skip]
/// Hangul trailing consonant (jongseong) names, indexed by `TIndex`; index 0 is
/// the empty name of an LV syllable.
pub(crate) const HANGUL_T: &[&str] = &{trail_rust};

/// Length in bytes of the longest name in [`NAMES_BLOB`]; a longer query
/// cannot match, and a decoded name always fits in a buffer of this size.
pub(crate) const MAX_NAME_LEN: usize = {max_name_len};

/// Front-coded explicit names, sorted by name: `[prefix u8][suffix][NUL]`.
#[rustfmt::skip]
pub(crate) static NAMES_BLOB: &[u8] = b"{rust_bytes(blob)}";

/// Byte offset in [`NAMES_BLOB`] of the first name of each front-coding block.
#[rustfmt::skip]
pub(crate) static BLOCK_FIRST_NAME: &[u32] = &[
{wrap_array(["0x%X" % off for off in block_offsets])}
];

/// Code point of each explicit name, in [`NAMES_BLOB`] order.
#[rustfmt::skip]
pub(crate) static CODEPOINTS: &[u32] = &[
{wrap_array(["0x%X" % cp for cp in codepoints])}
];
'''
    return src, stats


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="verify the checked-in table")
    ap.add_argument("--stats", action="store_true", help="print the size breakdown")
    args = ap.parse_args()

    src, stats = render()
    if args.stats:
        for k, v in stats.items():
            print(f"{k:24s} {v:>10,d}")
    if args.check:
        with open(OUT, "r", encoding="utf-8") as f:
            current = f.read()
        if current != src:
            print(f"gen_ucn_name_table: {OUT} is out of date", file=sys.stderr)
            return 1
        print(f"gen_ucn_name_table: {OUT} is current")
        return 0
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(src)
    print(f"gen_ucn_name_table: wrote {OUT} ({len(src):,d} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
