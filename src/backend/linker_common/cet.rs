//! GNU x86 property note (`.note.gnu.property`) merging.
//!
//! The kernel, glibc and any CET-enabled object carry 64-bit properties in a
//! `.note.gnu.property` note (type `NT_GNU_PROPERTY_TYPE_0` = 5).  At link
//! time GNU ld folds the per-object property sets into one output note using
//! class-dependent rules.  The reference is binutils 2.47:
//! `bfd/elfxx-x86.c::_bfd_x86_elf_merge_gnu_properties` for the merge,
//! `bfd/elfxx-x86.c::_bfd_x86_elf_link_setup_gnu_properties` for the
//! command-line creation path, `include/elf/common.h` for the class ranges
//! and bit numbers, and `ld/emulparams/x86-64-level.sh` for the ISA-level
//! command-line interface:
//!
//! * **AND class** — types `0xc0000002..=0xc0007fff`
//!   (`GNU_PROPERTY_X86_FEATURE_1_AND` = `0xc0000002`, the only member in
//!   practice): when every real input has the type the value is the AND of
//!   all of them, otherwise the base value is 0.  The `-z ibt` / `-z shstk` /
//!   `-z lam-u48` / `-z lam-u57` bits are ORed in afterwards (they can create
//!   the property even when no input had it).  A result of 0 drops the type.
//!   For 32-bit output the LAM bits are cleared first (bfd's x86 fixup:
//!   "Keep LAM features only for 64-bit output").
//! * **OR class** — types `0xc0008000..=0xc000ffff` plus
//!   `GNU_PROPERTY_X86_COMPAT_ISA_1_NEEDED` = `0xc0000001`
//!   (`GNU_PROPERTY_X86_FEATURE_1_NEEDED` = `0xc0008001`,
//!   `GNU_PROPERTY_X86_ISA_1_NEEDED` = `0xc0008002`): values are ORed
//!   across all inputs that have the type; the `-z x86-64-*` ISA bit (if
//!   any) is ORed in as well.  An all-zero result is REMOVED — binutils
//!   drops OR-class properties whose bits are all empty, so a value-0
//!   `ISA_1_NEEDED` never survives a GNU link.  (glibc's CRT carries the
//!   baseline note with value 1, `GNU_PROPERTY_X86_ISA_1_BASELINE` — not 0 —
//!   which is why real-world links keep it.)
//! * **OR-AND class** — types `0xc0010000..=0xc0017fff` plus
//!   `GNU_PROPERTY_X86_COMPAT_ISA_1_USED` = `0xc0000000`
//!   (`GNU_PROPERTY_X86_FEATURE_2_USED` = `0xc0010001`,
//!   `GNU_PROPERTY_X86_ISA_1_USED` = `0xc0010002`): kept only when every
//!   real input has the type, value is the OR of all inputs.  A value of 0
//!   is KEPT: bfd's x86 fixup removes empty AND / OR bitmasks only, and
//!   gas 2.47 `-mx86-used-note=yes` i386 objects carry `ISA_1_USED = 0`.
//! * **Generic types** (`bfd/elf-properties.c`): the `0xb0000000..=0xb0007fff`
//!   AND and `0xb0008000..=0xb000ffff` OR bitmask ranges follow the x86
//!   AND / OR rules (`GNU_PROPERTY_1_NEEDED` is the first OR type);
//!   `GNU_PROPERTY_STACK_SIZE` takes the maximum; the zero-sized
//!   `GNU_PROPERTY_NO_COPY_ON_PROTECTED` / `GNU_PROPERTY_MEMORY_SEAL`
//!   markers survive when any input has them.
//!
//! Types binutils does not know are dropped when the inputs are parsed
//! (bfd: "unsupported GNU_PROPERTY_TYPE" warning), so they neither veto nor
//! survive.  Every note of the section is parsed and a type repeated within
//! one object ORs (see [`parse_property_note_bytes`]).
//!
//! The ISA level IS a linker option, contrary to older claims in this file's
//! history: GNU ld spells it `-z x86-64-{baseline,v2,v3,v4}`
//! (`ld/emulparams/x86-64-level.sh`), default off.  It ORs the level bit
//! into `GNU_PROPERTY_X86_ISA_1_NEEDED` and creates the property when no
//! input had one.  There is one deliberate divergence: binutils 2.47 omits
//! `case 1:` (baseline) from the merge-time switch, so `-z x86-64-baseline`
//! against inputs that already carry `ISA_1_NEEDED` dies with a BFD
//! internal-error abort (measured on 2.44; the 2.47 source still lacks the
//! case).  lccc ORs the baseline bit like every other level instead of
//! reproducing the crash.
//!
//! "Every real input" means the objects read from files — never an object
//! the linker synthesizes itself (the build-id note, the `<string-merge>`
//! pool, the property carrier), which carries no note and would otherwise
//! veto every AND-class type.  The boundary is structural, not opt-in: the
//! caller passes the object count at the end of input loading, and every
//! synthetic object is appended after it.  (The earlier opt-in index set
//! missed the string-merge pool, so on distributions whose crt files and
//! compiler default carry CET notes — Ubuntu — every link that merged a
//! string silently lost `IBT|SHSTK`.)
//!
//! The merge is applied **to the input objects, before the section merge**:
//! the first real object that carried the section keeps the merged bytes,
//! all other copies are zeroed, and when no input had the section a
//! synthetic carrier object (see [`synthetic_property_object`]) is returned
//! for the caller to append.  This guarantees the layout, the section size
//! and the `PT_NOTE` / `PT_GNU_PROPERTY` program headers all reflect the
//! merged note — patching output sections after the merge does not work,
//! because the emitter refills section data from the input objects during
//! layout.
//!
//! Output note format (all fields little-endian):
//! ```text
//!   u32 namesz = 4
//!   u32 descsz
//!   u32 type   = 5                     (NT_GNU_PROPERTY_TYPE_0)
//!   "GNU\0"
//!   entries: ( u32 type, u32 datasz, data, zero padding )
//! ```
//! with the entries sorted by type ascending and each entry's data padded to
//! 8 bytes on 64-bit targets and 4 on 32-bit ones: a 4-byte bitmask entry is
//! 16 bytes in a 64-bit note and 12 in a 32-bit note.

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::SectionData;
use super::types::{Elf64Object, Elf64Section};

/// Section name carrying the 64-bit property note.
pub const PROPERTY_SECTION: &str = ".note.gnu.property";

/// `NT_GNU_PROPERTY_TYPE_0` — the note type gcc/bfd use for the x86
/// property note on 64-bit targets.
const NT_GNU_PROPERTY: u32 = 5;

/// `GNU_PROPERTY_X86_FEATURE_1_AND`.
const PROP_X86_FEATURE_1_AND: u32 = 0xc000_0002;
/// `GNU_PROPERTY_X86_ISA_1_NEEDED`.
const PROP_X86_ISA_1_NEEDED: u32 = 0xc000_8002;
/// `GNU_PROPERTY_X86_ISA_1_USED`
/// (`GNU_PROPERTY_X86_UINT32_OR_AND_LO + 2`).
const PROP_X86_ISA_1_USED: u32 = 0xc001_0002;
/// `GNU_PROPERTY_X86_COMPAT_ISA_1_USED` — below the class ranges, but
/// binutils merges it with OR-AND semantics.
const PROP_X86_COMPAT_ISA_1_USED: u32 = 0xc000_0000;
/// `GNU_PROPERTY_X86_COMPAT_ISA_1_NEEDED` — below the class ranges, but
/// binutils merges it with OR semantics.
const PROP_X86_COMPAT_ISA_1_NEEDED: u32 = 0xc000_0001;

/// Generic (processor-independent) property types from binutils 2.47
/// `include/elf/common.h`.
const GNU_PROPERTY_STACK_SIZE: u32 = 1;
const GNU_PROPERTY_NO_COPY_ON_PROTECTED: u32 = 2;
const GNU_PROPERTY_MEMORY_SEAL: u32 = 3;
/// Generic 4-byte bitmask ranges (`GNU_PROPERTY_UINT32_{AND,OR}_{LO,HI}`;
/// `GNU_PROPERTY_1_NEEDED` = `GNU_PROPERTY_UINT32_OR_LO`).
const GNU_AND_LO: u32 = 0xb000_0000;
const GNU_AND_HI: u32 = 0xb000_7fff;
const GNU_OR_LO: u32 = 0xb000_8000;
const GNU_OR_HI: u32 = 0xb000_ffff;

/// x86 class ranges from binutils 2.47 `include/elf/common.h`
/// (`GNU_PROPERTY_X86_UINT32_{AND,OR,OR_AND}_{LO,HI}`).
const AND_LO: u32 = 0xc000_0002;
const AND_HI: u32 = 0xc000_7fff;
const OR_LO: u32 = 0xc000_8000;
const OR_HI: u32 = 0xc000_ffff;
const OR_AND_LO: u32 = 0xc001_0000;
const OR_AND_HI: u32 = 0xc001_7fff;

/// `-z ibt` bit in `GNU_PROPERTY_X86_FEATURE_1_AND`
/// (`GNU_PROPERTY_X86_FEATURE_1_IBT`).
const FEATURE_1_IBT: u32 = 0x1;
/// `-z shstk` bit (`GNU_PROPERTY_X86_FEATURE_1_SHSTK`).
const FEATURE_1_SHSTK: u32 = 0x2;
/// `-z lam-u48` bits: binutils sets both
/// `GNU_PROPERTY_X86_FEATURE_1_LAM_U48` (bit 2) and
/// `GNU_PROPERTY_X86_FEATURE_1_LAM_U57` (bit 3).
const FEATURE_1_LAM_U48: u32 = 0x4 | 0x8;
/// `-z lam-u57` bit (`GNU_PROPERTY_X86_FEATURE_1_LAM_U57`, bit 3).
const FEATURE_1_LAM_U57: u32 = 0x8;

/// `GNU_PROPERTY_X86_ISA_1_BASELINE` (bit 0).
const ISA_1_BASELINE: u32 = 0x1;
/// `GNU_PROPERTY_X86_ISA_1_V2` (bit 1).
const ISA_1_V2: u32 = 0x2;
/// `GNU_PROPERTY_X86_ISA_1_V3` (bit 2).
const ISA_1_V3: u32 = 0x4;
/// `GNU_PROPERTY_X86_ISA_1_V4` (bit 3).
const ISA_1_V4: u32 = 0x8;

/// `-z` options that influence the property note, mirroring the x86 subset
/// of binutils' `struct elf_linker_x86_params` that the merge consults.
#[derive(Debug, Clone, Copy, Default)]
pub struct PropertyLinkFlags {
    pub ibt: bool,
    pub shstk: bool,
    pub lam_u48: bool,
    pub lam_u57: bool,
    /// `-z x86-64-{baseline,v2,v3,v4}` ISA level: 0 = unset (the default —
    /// nothing is injected), 1 = baseline, 2/3/4 = v2/v3/v4.  ORed into
    /// `GNU_PROPERTY_X86_ISA_1_NEEDED`, creating the property when no input
    /// had one, exactly like binutils' `params.isa_level`.
    pub isa_level: u32,
}

impl PropertyLinkFlags {
    fn feature1_bits(&self) -> u32 {
        let mut v = 0;
        if self.ibt {
            v |= FEATURE_1_IBT;
        }
        if self.shstk {
            v |= FEATURE_1_SHSTK;
        }
        if self.lam_u48 {
            v |= FEATURE_1_LAM_U48;
        }
        if self.lam_u57 {
            v |= FEATURE_1_LAM_U57;
        }
        v
    }

    /// The `GNU_PROPERTY_X86_ISA_1_*` bit for [`Self::isa_level`], or 0 when
    /// unset.  Level 1 yields the baseline bit: binutils aborts the merge in
    /// that combination (missing `case 1:`), which is a bug, not a rule.
    fn isa_bit(&self) -> u32 {
        match self.isa_level {
            1 => ISA_1_BASELINE,
            2 => ISA_1_V2,
            3 => ISA_1_V3,
            4 => ISA_1_V4,
            _ => 0,
        }
    }
}

/// Which merge class a property type belongs to.  Classification is by
/// range, exactly as in binutils 2.47 (`elf-properties.c` for the generic
/// types, `_bfd_x86_elf_merge_gnu_properties` for the x86 processor range)
/// — matching on the handful of known constants instead silently
/// mis-merges every other type in the ranges.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PropertyClass {
    /// 4-byte bitmask, kept only when every input has it, ANDed.
    And,
    /// 4-byte bitmask, kept when any input has it, ORed.
    Or,
    /// 4-byte bitmask, kept only when every input has it, ORed (x86 "used"
    /// notes).
    OrAnd,
    /// `GNU_PROPERTY_STACK_SIZE`: address-sized, maximum over the inputs
    /// that have it.
    Max,
    /// Zero-sized marker (`GNU_PROPERTY_NO_COPY_ON_PROTECTED`,
    /// `GNU_PROPERTY_MEMORY_SEAL`): kept when any input has it.
    Marker,
}

/// The merge class of `t`, or `None` for a type binutils does not
/// understand.  bfd warns ("unsupported GNU_PROPERTY_TYPE") and drops such
/// entries at parse time — they never reach the merge, so an unknown type
/// can neither veto nor survive; lccc does the same.
fn classify(t: u32) -> Option<PropertyClass> {
    Some(match t {
        GNU_PROPERTY_STACK_SIZE => PropertyClass::Max,
        GNU_PROPERTY_NO_COPY_ON_PROTECTED | GNU_PROPERTY_MEMORY_SEAL => PropertyClass::Marker,
        GNU_AND_LO..=GNU_AND_HI => PropertyClass::And,
        GNU_OR_LO..=GNU_OR_HI => PropertyClass::Or,
        PROP_X86_COMPAT_ISA_1_USED => PropertyClass::OrAnd,
        PROP_X86_COMPAT_ISA_1_NEEDED => PropertyClass::Or,
        AND_LO..=AND_HI => PropertyClass::And,
        OR_LO..=OR_HI => PropertyClass::Or,
        OR_AND_LO..=OR_AND_HI => PropertyClass::OrAnd,
        _ => return None,
    })
}

/// Per-type merge of the property classes.
///
/// `inputs` is one map per *real* input object (type → value); objects
/// without a property note contribute an empty map, which makes them veto
/// AND / OR-AND types.  `is_32` is the output ELF class (LAM fixup).
///
/// Zero results: binutils removes an all-zero AND or OR bitmask (generic
/// `elf_merge_gnu_properties`, x86 `_bfd_x86_elf_link_fixup_gnu_properties`)
/// but deliberately KEEPS an all-zero OR-AND "used" bitmask — "every input
/// was built with used-notes and none used an ISA extension" is information
/// (i386 objects from gas `-mx86-used-note=yes` carry `ISA_1_USED = 0`).
fn merge_properties(
    inputs: &[FxHashMap<u32, u64>],
    flags: &PropertyLinkFlags,
    is_32: bool,
) -> FxHashMap<u32, u64> {
    let mut all_types: FxHashSet<u32> = FxHashSet::default();
    for m in inputs {
        all_types.extend(m.keys());
    }
    // The `-z` option bits and the ISA level can create the property even
    // when no input carried the note at all (GNU ld creates the entry from
    // the command line alone), so seed the type set with the types those
    // options target.
    if flags.feature1_bits() != 0 {
        all_types.insert(PROP_X86_FEATURE_1_AND);
    }
    if flags.isa_bit() != 0 {
        all_types.insert(PROP_X86_ISA_1_NEEDED);
    }

    let mut merged: FxHashMap<u32, u64> = FxHashMap::default();
    for t in all_types {
        // Parsing never admits an unclassified type; the seeds are
        // classified by construction.
        let Some(class) = classify(t) else { continue };
        // `all` is vacuously true for an empty input list, but "every
        // input has the type" is meaningless when there are no inputs:
        // without the guard `-z shstk` with no inputs would AND over
        // nothing and produce FEATURE_1_AND = 0xffff_ffff.
        let all_have = !inputs.is_empty() && inputs.iter().all(|m| m.contains_key(&t));
        let present = || inputs.iter().filter_map(|m| m.get(&t).copied());
        match class {
            PropertyClass::And => {
                // AND of all inputs when every input has the type,
                // otherwise a 0 base; then the -z feature bits (only
                // FEATURE_1_AND has command-line bits).
                let mut value = if all_have {
                    present().fold(u64::from(u32::MAX), |a, b| a & b)
                } else {
                    0
                };
                if t == PROP_X86_FEATURE_1_AND {
                    value |= u64::from(flags.feature1_bits());
                    // bfd's x86 fixup: "Keep LAM features only for 64-bit
                    // output."  (bfd clears the bits after its zero check
                    // and would emit FEATURE_1_AND = 0 for a LAM-only
                    // i386 input set; an all-zero AND bitmask means
                    // nothing, so lccc applies the removal rule to the
                    // final value instead.)
                    if is_32 {
                        value &= !u64::from(FEATURE_1_LAM_U48 | FEATURE_1_LAM_U57);
                    }
                }
                if value != 0 {
                    merged.insert(t, value);
                }
            }
            PropertyClass::Or => {
                // OR of whoever has the type, plus the -z ISA bit for
                // ISA_1_NEEDED.  An all-zero result is removed (a seeded
                // ISA_1_NEEDED always carries the level bit, so the
                // create-from-command-line case still inserts).
                let mut value = present().fold(0, |a, b| a | b);
                if t == PROP_X86_ISA_1_NEEDED {
                    value |= u64::from(flags.isa_bit());
                }
                if value != 0 {
                    merged.insert(t, value);
                }
            }
            PropertyClass::OrAnd => {
                // Kept only when every input has the type; ORed; a 0
                // result is KEPT (see above).
                if all_have {
                    merged.insert(t, present().fold(0, |a, b| a | b));
                }
            }
            PropertyClass::Max => {
                if let Some(v) = present().max() {
                    merged.insert(t, v);
                }
            }
            PropertyClass::Marker => {
                if present().next().is_some() {
                    merged.insert(t, 0);
                }
            }
        }
    }
    merged
}

/// The property-entry data size of `t` in an output of class `is_32`.
fn property_datasz(t: u32, is_32: bool) -> usize {
    match classify(t) {
        Some(PropertyClass::Max) => {
            if is_32 {
                4
            } else {
                8
            }
        }
        Some(PropertyClass::Marker) => 0,
        _ => 4,
    }
}

/// Parse the property note of one object; `None` when the object has no
/// (or an empty / malformed) `.note.gnu.property` section.
fn parse_property_note(obj: &Elf64Object, is_32: bool) -> Option<FxHashMap<u32, u64>> {
    let si = obj
        .sections
        .iter()
        .position(|s| s.name == PROPERTY_SECTION)?;
    parse_property_note_bytes(
        obj.section_data[si].as_slice(),
        obj.sections[si].addralign,
        is_32,
    )
}

fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// [`parse_property_note`] on the raw bytes of a `.note.gnu.property`
/// section with alignment `sh_addralign`; shared with backends that keep
/// their own object model.
///
/// Mirrors binutils 2.47 `elf.c::elf_parse_notes` +
/// `elf-properties.c::_bfd_elf_parse_gnu_properties`:
///
/// * **Every** note in the section is read, not just the first.  gas 2.47
///   defaults to `-mx86-used-note=yes` and appends its own
///   `NT_GNU_PROPERTY_TYPE_0` note (FEATURE_2_USED / ISA_1_USED) after a
///   hand-written one in the same section; reading only the first note lost
///   those types, and the object then vetoed them for the whole link.
/// * Notes are laid out at the section alignment (`< 4` counts as 4; any
///   other value than 4 or 8 makes bfd skip the section): the descriptor
///   starts at `align_up(12 + namesz, align)`, the next note at
///   `align_up(desc + descsz, align)`.
/// * Property entries inside a descriptor step by the ELF-class alignment
///   (8 on ELFCLASS64, 4 on ELFCLASS32: `ptr += align_up(datasz,
///   align_size)`), so a 4-byte bitmask occupies 16 bytes in a 64-bit note
///   and 12 in a 32-bit one.
/// * A type that appears twice within one object — two entries or two
///   notes — ORs its bitmask (`prop->u.number |= ...`).
/// * Unknown types are skipped; a known type with the wrong `datasz`, or a
///   descriptor overrunning its note, makes the object's property set
///   invalid (bfd: "corrupt ... size", `elf_properties (abfd) = NULL`), which
///   lccc reports as `None`: the object then vetoes AND / OR-AND types.
fn parse_property_note_bytes(
    data: &[u8],
    sh_addralign: u64,
    is_32: bool,
) -> Option<FxHashMap<u32, u64>> {
    let align = match sh_addralign {
        0..=4 => 4usize,
        8 => 8usize,
        _ => return None,
    };
    let entry_align = if is_32 { 4usize } else { 8usize };
    let align_up = |v: usize, a: usize| v.checked_add(a - 1).map(|x| x & !(a - 1));
    let mut map: FxHashMap<u32, u64> = FxHashMap::default();
    let mut p = 0usize;
    while p < data.len() {
        let namesz = read_u32(data, p)? as usize;
        let descsz = read_u32(data, p + 4)? as usize;
        let ntype = read_u32(data, p + 8)?;
        let name = data.get(p + 12..(p + 12).checked_add(namesz)?)?;
        let desc_off = p.checked_add(align_up(12 + namesz, align)?)?;
        let desc = data.get(desc_off..desc_off.checked_add(descsz)?)?;
        p = desc_off.checked_add(align_up(descsz, align)?)?;
        if ntype != NT_GNU_PROPERTY || name != b"GNU\0" {
            continue;
        }
        if descsz < 8 || descsz % entry_align != 0 {
            return None;
        }
        let mut o = 0usize;
        while o < descsz {
            let t = read_u32(desc, o)?;
            let datasz = read_u32(desc, o + 4)? as usize;
            let body = desc.get(o + 8..(o + 8).checked_add(datasz)?)?;
            o = (o + 8).checked_add(align_up(datasz, entry_align)?)?;
            let Some(class) = classify(t) else { continue };
            if datasz != property_datasz(t, is_32) {
                return None;
            }
            match class {
                PropertyClass::Max => {
                    let v = if is_32 {
                        u64::from(read_u32(body, 0)?)
                    } else {
                        u64::from_le_bytes(body.try_into().ok()?)
                    };
                    map.insert(t, v);
                }
                PropertyClass::Marker => {
                    map.insert(t, 0);
                }
                _ => {
                    *map.entry(t).or_insert(0) |= u64::from(read_u32(body, 0)?);
                }
            }
        }
        if o != descsz {
            return None;
        }
    }
    (!map.is_empty()).then_some(map)
}

/// One real input's `.note.gnu.property` section as seen by
/// [`merge_property_notes`]: its bytes and `sh_addralign`.
pub type PropertySection<'a> = (&'a [u8], u64);

/// The merged `.note.gnu.property` of a link whose real inputs carry the
/// sections `notes` (`None`: the input has no such section), or `None`
/// when the merge leaves no property.  For backends with their own object
/// model (the ELF32 i386 linker); [`merge_property_into_objects`] is the
/// same merge applied to `Elf64Object`s.
pub fn merge_property_notes(
    notes: &[Option<PropertySection<'_>>],
    flags: &PropertyLinkFlags,
    is_32: bool,
) -> Option<Vec<u8>> {
    let inputs: Vec<FxHashMap<u32, u64>> = notes
        .iter()
        .map(|n| {
            n.and_then(|(d, a)| parse_property_note_bytes(d, a, is_32))
                .unwrap_or_default()
        })
        .collect();
    let merged = merge_properties(&inputs, flags, is_32);
    (!merged.is_empty()).then(|| build_property_note(&merged, is_32))
}

/// Build the merged note bytes in the canonical layout: one note, entries
/// sorted by type ascending, each entry's data padded to the ELF-class
/// alignment (see [`parse_property_note_bytes`]).
fn build_property_note(merged: &FxHashMap<u32, u64>, is_32: bool) -> Vec<u8> {
    let mut types: Vec<u32> = merged.keys().copied().collect();
    types.sort_unstable();
    let entry_align = if is_32 { 4 } else { 8 };
    let padded = |n: usize| (n + entry_align - 1) & !(entry_align - 1);
    let descsz: usize = types
        .iter()
        .map(|&t| 8 + padded(property_datasz(t, is_32)))
        .sum();
    let mut out = Vec::with_capacity(12 + 4 + descsz);
    out.extend_from_slice(&4u32.to_le_bytes()); // namesz
    out.extend_from_slice(&(descsz as u32).to_le_bytes());
    out.extend_from_slice(&NT_GNU_PROPERTY.to_le_bytes());
    out.extend_from_slice(b"GNU\0");
    for &t in &types {
        let datasz = property_datasz(t, is_32);
        out.extend_from_slice(&t.to_le_bytes());
        out.extend_from_slice(&(datasz as u32).to_le_bytes());
        let v = merged[&t];
        match datasz {
            0 => {}
            4 => out.extend_from_slice(&(v as u32).to_le_bytes()),
            _ => out.extend_from_slice(&v.to_le_bytes()),
        }
        out.resize(out.len() + padded(datasz) - datasz, 0);
    }
    out
}

/// A synthetic object carrying a single-section property note, mirroring
/// [`super::build_id::synthetic_note_object`]: section 0 is the NULL
/// placeholder, section 1 is the note.  The note alignment follows the
/// output class (8 on 64-bit, 4 on 32-bit), matching binutils.
fn synthetic_property_object(note: &[u8], is_32: bool) -> Elf64Object {
    let size = note.len() as u64;
    let sections = vec![
        Elf64Section {
            name_idx: 0,
            name: String::new(),
            sh_type: 0,
            flags: 0,
            addr: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            addralign: 0,
            entsize: 0,
        },
        Elf64Section {
            name_idx: 0,
            name: PROPERTY_SECTION.into(),
            sh_type: 7, // SHT_NOTE
            flags: 0x2, // SHF_ALLOC
            addr: 0,
            offset: 0,
            size,
            link: 0,
            info: 0,
            addralign: if is_32 { 4 } else { 8 },
            entsize: 0,
        },
    ];
    let section_data = vec![SectionData::empty(), SectionData::owned(note.to_vec())];
    Elf64Object {
        sections,
        symbols: Vec::new(),
        section_data,
        relocations: vec![Vec::new(); 2],
        source_name: "<property>".into(),
    }
}

fn clear_property_section(obj: &mut Elf64Object) {
    if let Some(si) = obj.sections.iter().position(|s| s.name == PROPERTY_SECTION) {
        obj.section_data[si] = SectionData::empty();
        obj.sections[si].size = 0;
    }
}

/// Fold every real input's `.note.gnu.property` into a single note and
/// rewrite the *input objects* in place: the first real carrier keeps the
/// merged bytes, all other copies are cleared.  When no input carried the
/// section but `-z` options (CET bits or `-z x86-64-*`) still require one, a
/// synthetic carrier object is returned for the caller to append (mirroring
/// [`super::build_id::synthetic_note_object`]) so the section merge emits
/// the note.
///
/// Only `objects[..real_inputs]` take part: `real_inputs` is the object
/// count at the end of input loading, and everything after it is
/// linker-synthesized (build-id note, string-merge pool, ...) and must
/// neither veto an AND-class type nor carry the merged bytes.
/// `is_32` is the link's ELF class: property entries are 16 bytes on
/// 64-bit targets and 12 on 32-bit ones (see [`parse_property_note`]).
pub fn merge_property_into_objects(
    objects: &mut [Elf64Object],
    real_inputs: usize,
    flags: &PropertyLinkFlags,
    is_32: bool,
) -> Result<Option<Elf64Object>, String> {
    if real_inputs > objects.len() {
        return Err(format!(
            "internal error: property merge over {real_inputs} inputs, only {} objects",
            objects.len()
        ));
    }
    let real = &mut objects[..real_inputs];
    let notes: Vec<Option<PropertySection<'_>>> = real
        .iter()
        .map(|o| {
            o.sections
                .iter()
                .position(|s| s.name == PROPERTY_SECTION)
                .map(|si| (o.section_data[si].as_slice(), o.sections[si].addralign))
        })
        .collect();
    let note = merge_property_notes(&notes, flags, is_32);

    let first_carrier = real
        .iter()
        .position(|o| o.sections.iter().any(|s| s.name == PROPERTY_SECTION));

    match (note.as_ref(), first_carrier) {
        (Some(n), Some(i)) => {
            let o = &mut real[i];
            let si = o
                .sections
                .iter()
                .position(|s| s.name == PROPERTY_SECTION)
                .unwrap();
            o.section_data[si] = SectionData::owned(n.clone());
            o.sections[si].size = n.len() as u64;
            o.sections[si].addralign = o.sections[si].addralign.max(if is_32 { 4 } else { 8 });
            for (j, other) in real.iter_mut().enumerate() {
                if j != i {
                    clear_property_section(other);
                }
            }
            Ok(None)
        }
        (Some(n), None) => Ok(Some(synthetic_property_object(n, is_32))),
        (None, _) => {
            // The merge cleared the property (e.g. AND type present in
            // some inputs only, no -z bits): drop every copy so no note
            // remains in the output.
            for other in real.iter_mut() {
                clear_property_section(other);
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: &[(u32, u64)]) -> FxHashMap<u32, u64> {
        let mut map = FxHashMap::default();
        for &(t, v) in pairs {
            map.insert(t, v);
        }
        map
    }

    fn flags() -> PropertyLinkFlags {
        PropertyLinkFlags::default()
    }

    #[test]
    fn and_class_intersects_and_vetoes() {
        // AND of all inputs when every input has the type ...
        let got = merge_properties(
            &[
                m(&[(PROP_X86_FEATURE_1_AND, 0x3)]),
                m(&[(PROP_X86_FEATURE_1_AND, 0x1)]),
            ],
            &flags(),
            false,
        );
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&0x1));
        // ... and nothing (base 0, dropped) when any input lacks it.
        let got = merge_properties(
            &[m(&[(PROP_X86_FEATURE_1_AND, 0x3)]), m(&[])],
            &flags(),
            false,
        );
        assert!(!got.contains_key(&PROP_X86_FEATURE_1_AND));
    }

    #[test]
    fn and_class_z_bits_rescue_and_create() {
        // `-z ibt` rescues a vetoed AND type (IBT only, not the input bits).
        let f = PropertyLinkFlags {
            ibt: true,
            ..flags()
        };
        let got = merge_properties(&[m(&[(PROP_X86_FEATURE_1_AND, 0x2)]), m(&[])], &f, false);
        assert_eq!(
            got.get(&PROP_X86_FEATURE_1_AND),
            Some(&u64::from(FEATURE_1_IBT))
        );
        // `-z shstk` creates the property with no inputs at all.
        let f = PropertyLinkFlags {
            shstk: true,
            ..flags()
        };
        let got = merge_properties(&[], &f, false);
        assert_eq!(
            got.get(&PROP_X86_FEATURE_1_AND),
            Some(&u64::from(FEATURE_1_SHSTK))
        );
        // `-z lam-u48` sets both LAM bits, like binutils.
        let f = PropertyLinkFlags {
            lam_u48: true,
            ..flags()
        };
        let got = merge_properties(&[], &f, false);
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&0xc));
    }

    #[test]
    fn and_rule_applies_to_the_whole_and_range() {
        // A hypothetical second AND-range type must be ANDed, not ORed:
        // classification is by range, not by constant.
        let t = 0xc000_0003u32;
        assert_eq!(classify(t), Some(PropertyClass::And));
        let got = merge_properties(&[m(&[(t, 0b110)]), m(&[(t, 0b101)])], &flags(), false);
        assert_eq!(got.get(&t), Some(&0b100));
        let got = merge_properties(&[m(&[(t, 0b110)]), m(&[])], &flags(), false);
        assert!(!got.contains_key(&t));
    }

    #[test]
    fn or_class_ors_and_drops_zero() {
        // OR accumulation across inputs ...
        let got = merge_properties(
            &[
                m(&[(PROP_X86_ISA_1_NEEDED, 0x1)]),
                m(&[(PROP_X86_ISA_1_NEEDED, 0x4)]),
            ],
            &flags(),
            false,
        );
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&0x5));
        // ... but an all-zero OR result is REMOVED (binutils drops
        // OR-class properties whose bits are all empty).
        let got = merge_properties(&[m(&[(PROP_X86_ISA_1_NEEDED, 0x0)])], &flags(), false);
        assert!(!got.contains_key(&PROP_X86_ISA_1_NEEDED));
        // FEATURE_1_NEEDED shares the OR rule.
        let t = 0xc000_8001u32;
        let got = merge_properties(&[m(&[(t, 0x1)]), m(&[])], &flags(), false);
        assert_eq!(got.get(&t), Some(&0x1));
    }

    #[test]
    fn compat_needed_is_or_class() {
        // 0xc0000001 sits below the ranges but binutils merges it with OR
        // semantics: kept when ANY input has it.
        assert_eq!(
            classify(PROP_X86_COMPAT_ISA_1_NEEDED),
            Some(PropertyClass::Or)
        );
        let got = merge_properties(
            &[m(&[(PROP_X86_COMPAT_ISA_1_NEEDED, 0x2)]), m(&[])],
            &flags(),
            false,
        );
        assert_eq!(got.get(&PROP_X86_COMPAT_ISA_1_NEEDED), Some(&0x2));
    }

    #[test]
    fn or_and_class_requires_unanimity_and_keeps_zero() {
        let t = PROP_X86_ISA_1_USED;
        let got = merge_properties(&[m(&[(t, 0x3)]), m(&[(t, 0x5)])], &flags(), false);
        assert_eq!(got.get(&t), Some(&0x7));
        // One input without the type vetoes it ...
        let got = merge_properties(&[m(&[(t, 0x3)]), m(&[])], &flags(), false);
        assert!(!got.contains_key(&t));
        // ... but an all-zero OR is KEPT: bfd's x86 fixup removes empty
        // AND / OR bitmasks only (gas 2.47 i386 used-notes carry
        // ISA_1_USED = 0, and GNU ld keeps it).
        let got = merge_properties(&[m(&[(t, 0x0)]), m(&[(t, 0x0)])], &flags(), true);
        assert_eq!(got.get(&t), Some(&0x0));
        let got = merge_properties(&[m(&[(PROP_X86_COMPAT_ISA_1_USED, 0)])], &flags(), false);
        assert_eq!(got.get(&PROP_X86_COMPAT_ISA_1_USED), Some(&0x0));
    }

    #[test]
    fn compat_used_is_or_and_class() {
        assert_eq!(
            classify(PROP_X86_COMPAT_ISA_1_USED),
            Some(PropertyClass::OrAnd)
        );
        let got = merge_properties(
            &[
                m(&[(PROP_X86_COMPAT_ISA_1_USED, 0x1)]),
                m(&[(PROP_X86_COMPAT_ISA_1_USED, 0x2)]),
            ],
            &flags(),
            false,
        );
        assert_eq!(got.get(&PROP_X86_COMPAT_ISA_1_USED), Some(&0x3));
        let got = merge_properties(
            &[m(&[(PROP_X86_COMPAT_ISA_1_USED, 0x1)]), m(&[])],
            &flags(),
            false,
        );
        assert!(!got.contains_key(&PROP_X86_COMPAT_ISA_1_USED));
    }

    #[test]
    fn isa_level_injects_and_creates() {
        // `-z x86-64-v3` with no inputs creates ISA_1_NEEDED = V3.
        let f = PropertyLinkFlags {
            isa_level: 3,
            ..flags()
        };
        let got = merge_properties(&[], &f, false);
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&u64::from(ISA_1_V3)));
        // With inputs the level bit is ORed in (baseline | v2).
        let f = PropertyLinkFlags {
            isa_level: 2,
            ..flags()
        };
        let got = merge_properties(&[m(&[(PROP_X86_ISA_1_NEEDED, 0x1)])], &f, false);
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&0x3));
        // Baseline ORs the baseline bit instead of aborting the merge
        // (binutils 2.47 BFD-internal-errors here — missing `case 1:`).
        let f = PropertyLinkFlags {
            isa_level: 1,
            ..flags()
        };
        let got = merge_properties(&[m(&[(PROP_X86_ISA_1_NEEDED, 0x2)])], &f, false);
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&0x3));
        // Levels outside 1..=4 inject nothing (the CLI layer rejects them;
        // the merge must still be total).
        let f = PropertyLinkFlags {
            isa_level: 9,
            ..flags()
        };
        let got = merge_properties(&[], &f, false);
        assert!(got.is_empty());
    }

    #[test]
    fn unknown_types_are_dropped_at_parse() {
        // bfd warns "unsupported GNU_PROPERTY_TYPE" and ignores the entry,
        // so an unknown type neither vetoes nor survives: a note holding
        // IBT plus an unknown type merges with an IBT-only note to IBT.
        assert_eq!(classify(0x1234_5678), None);
        assert_eq!(classify(0xc002_0000), None); // x86 range gap
        assert_eq!(classify(0xe000_0000), None); // GNU_PROPERTY_LOUSER
        let mut desc = Vec::new();
        for (t, v) in [(PROP_X86_FEATURE_1_AND, 1u32), (0xc002_0000, 7)] {
            desc.extend_from_slice(&t.to_le_bytes());
            desc.extend_from_slice(&4u32.to_le_bytes());
            desc.extend_from_slice(&v.to_le_bytes());
            desc.extend_from_slice(&[0; 4]);
        }
        let with_unknown = raw_note(5, b"GNU\0", &desc, 8);
        assert_eq!(
            parse_property_note_bytes(&with_unknown, 8, false),
            Some(m(&[(PROP_X86_FEATURE_1_AND, 1)]))
        );
        let plain = build_property_note(&m(&[(PROP_X86_FEATURE_1_AND, 1)]), false);
        let got = merge_property_notes(
            &[Some((&with_unknown, 8)), Some((&plain, 8))],
            &flags(),
            false,
        );
        assert_eq!(got, Some(plain));
    }

    /// One note with the given header fields, descriptor padded to `align`.
    fn raw_note(ntype: u32, name: &[u8], desc: &[u8], align: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(&(desc.len() as u32).to_le_bytes());
        out.extend_from_slice(&ntype.to_le_bytes());
        out.extend_from_slice(name);
        while out.len() % align != 0 {
            out.push(0);
        }
        out.extend_from_slice(desc);
        while out.len() % align != 0 {
            out.push(0);
        }
        out
    }

    /// A 32-bit-class bitmask entry list (12-byte entries).
    fn desc32(pairs: &[(u32, u32)]) -> Vec<u8> {
        let mut d = Vec::new();
        for &(t, v) in pairs {
            d.extend_from_slice(&t.to_le_bytes());
            d.extend_from_slice(&4u32.to_le_bytes());
            d.extend_from_slice(&v.to_le_bytes());
        }
        d
    }

    #[test]
    fn every_note_in_the_section_is_parsed_and_repeats_or() {
        // The layout gas 2.47 produces for a hand-written i386 note: the
        // source note first, then gas's own used-note appended in the same
        // section.  Reading only the first note lost FEATURE_2_USED /
        // ISA_1_USED, and the object then vetoed both for the whole link.
        let first = raw_note(5, b"GNU\0", &desc32(&[(PROP_X86_FEATURE_1_AND, 3)]), 4);
        let used = raw_note(
            5,
            b"GNU\0",
            &desc32(&[(0xc001_0001, 1), (PROP_X86_ISA_1_USED, 0)]),
            4,
        );
        // A foreign note between them is skipped, not a parse stop.
        let foreign = raw_note(3, b"GNU\0", &[0xaa; 20], 4);
        let sec = [first, foreign, used].concat();
        assert_eq!(
            parse_property_note_bytes(&sec, 4, true),
            Some(m(&[
                (PROP_X86_FEATURE_1_AND, 3),
                (0xc001_0001, 1),
                (PROP_X86_ISA_1_USED, 0),
            ]))
        );
        // The same type twice within one object ORs, across notes and
        // within one descriptor (bfd: `prop->u.number |= ...`).
        let again = raw_note(
            5,
            b"GNU\0",
            &desc32(&[(PROP_X86_ISA_1_NEEDED, 1), (PROP_X86_ISA_1_NEEDED, 4)]),
            4,
        );
        let second = raw_note(5, b"GNU\0", &desc32(&[(PROP_X86_ISA_1_NEEDED, 2)]), 4);
        assert_eq!(
            parse_property_note_bytes(&[again, second].concat(), 4, true),
            Some(m(&[(PROP_X86_ISA_1_NEEDED, 7)]))
        );
    }

    #[test]
    fn note_layout_follows_the_section_alignment() {
        // An ELF64 note section aligned to 4 packs notes at 4 even though
        // the property entries inside still step by 8 (bfd: note alignment
        // from sh_addralign, entry alignment from the ELF class).  The
        // 5-byte name makes the difference visible: the first note's
        // descriptor starts at 20 (not 24) and the next note at 24.
        let mut desc = Vec::new();
        desc.extend_from_slice(&PROP_X86_FEATURE_1_AND.to_le_bytes());
        desc.extend_from_slice(&4u32.to_le_bytes());
        desc.extend_from_slice(&2u32.to_le_bytes());
        desc.extend_from_slice(&[0; 4]);
        let odd = raw_note(1, b"ABCD\0", &[1, 2, 3, 4], 4);
        assert_eq!(odd.len(), 24);
        let sec = [odd, raw_note(5, b"GNU\0", &desc, 4)].concat();
        assert_eq!(
            parse_property_note_bytes(&sec, 4, false),
            Some(m(&[(PROP_X86_FEATURE_1_AND, 2)]))
        );
        // Read at 8-alignment the same bytes misparse into garbage, which
        // is exactly why the section alignment must be honoured.
        assert_ne!(
            parse_property_note_bytes(&sec, 8, false),
            Some(m(&[(PROP_X86_FEATURE_1_AND, 2)]))
        );
        // Alignments other than 0..=4 and 8 make bfd skip the section.
        assert_eq!(parse_property_note_bytes(&sec, 16, false), None);
    }

    #[test]
    fn corrupt_entries_invalidate_the_object() {
        // A bitmask type with datasz 8 is corrupt: bfd clears the object's
        // whole property set, so it then vetoes AND-class types.
        let mut desc = Vec::new();
        desc.extend_from_slice(&PROP_X86_FEATURE_1_AND.to_le_bytes());
        desc.extend_from_slice(&8u32.to_le_bytes());
        desc.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 0]);
        let bad = raw_note(5, b"GNU\0", &desc, 8);
        assert_eq!(parse_property_note_bytes(&bad, 8, false), None);
        // A descriptor that is not a multiple of the entry alignment, and
        // a truncated note, are corrupt too.
        let short = raw_note(5, b"GNU\0", &[0; 12], 4);
        assert_eq!(parse_property_note_bytes(&short, 4, false), None);
        let good = build_property_note(&m(&[(PROP_X86_FEATURE_1_AND, 3)]), false);
        assert_eq!(
            parse_property_note_bytes(&good[..good.len() - 1], 8, false),
            None
        );
    }

    #[test]
    fn lam_bits_survive_only_in_64_bit_output() {
        let inputs = [m(&[(PROP_X86_FEATURE_1_AND, 0xf)])];
        let got = merge_properties(&inputs, &flags(), false);
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&0xf));
        let got = merge_properties(&inputs, &flags(), true);
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&0x3));
        // A LAM-only set leaves nothing on i386: the empty AND bitmask
        // is removed rather than emitted as 0.
        let got = merge_properties(&[m(&[(PROP_X86_FEATURE_1_AND, 0xc)])], &flags(), true);
        assert!(!got.contains_key(&PROP_X86_FEATURE_1_AND));
    }

    #[test]
    fn generic_property_types_follow_elf_properties_c() {
        // GNU_PROPERTY_1_NEEDED (generic OR range): kept if any has it.
        let needed = GNU_OR_LO;
        let got = merge_properties(&[m(&[(needed, 1)]), m(&[])], &flags(), false);
        assert_eq!(got.get(&needed), Some(&1));
        // Generic AND range: vetoed by a missing input.
        let got = merge_properties(&[m(&[(GNU_AND_LO, 1)]), m(&[])], &flags(), false);
        assert!(!got.contains_key(&GNU_AND_LO));
        // STACK_SIZE: maximum of those that have it.
        let got = merge_properties(
            &[
                m(&[(GNU_PROPERTY_STACK_SIZE, 0x1000)]),
                m(&[]),
                m(&[(GNU_PROPERTY_STACK_SIZE, 0x8000)]),
            ],
            &flags(),
            false,
        );
        assert_eq!(got.get(&GNU_PROPERTY_STACK_SIZE), Some(&0x8000));
        // Zero-sized markers: present if any input has them.
        let got = merge_properties(
            &[m(&[(GNU_PROPERTY_NO_COPY_ON_PROTECTED, 0)]), m(&[])],
            &flags(),
            false,
        );
        assert_eq!(got.get(&GNU_PROPERTY_NO_COPY_ON_PROTECTED), Some(&0));
        // Encoding: STACK_SIZE is address-sized, markers have datasz 0,
        // and both round-trip through the parser in both classes.
        for is_32 in [false, true] {
            let merged = m(&[
                (GNU_PROPERTY_STACK_SIZE, 0x8000),
                (GNU_PROPERTY_MEMORY_SEAL, 0),
                (PROP_X86_FEATURE_1_AND, 3),
            ]);
            let note = build_property_note(&merged, is_32);
            let entries = if is_32 { 12 + 8 + 12 } else { 16 + 8 + 16 };
            assert_eq!(note.len(), 16 + entries);
            assert_eq!(note[4..8], (entries as u32).to_le_bytes());
            let a = if is_32 { 4 } else { 8 };
            assert_eq!(parse_property_note_bytes(&note, a, is_32), Some(merged));
        }
    }

    #[test]
    fn class_boundaries_match_binutils_2_47_common_h() {
        use PropertyClass::*;
        assert_eq!(classify(0xc000_0000), Some(OrAnd));
        assert_eq!(classify(0xc000_0001), Some(Or));
        assert_eq!(classify(0xc000_0002), Some(And));
        assert_eq!(classify(0xc000_7fff), Some(And));
        assert_eq!(classify(0xc000_8000), Some(Or));
        assert_eq!(classify(0xc000_ffff), Some(Or));
        assert_eq!(classify(0xc001_0000), Some(OrAnd));
        assert_eq!(classify(0xc001_7fff), Some(OrAnd));
        assert_eq!(classify(0xc001_8000), None);
        assert_eq!(classify(0xb000_0000), Some(And));
        assert_eq!(classify(0xb000_7fff), Some(And));
        assert_eq!(classify(0xb000_8000), Some(Or));
        assert_eq!(classify(0xb000_ffff), Some(Or));
        assert_eq!(classify(0xb001_0000), None);
        assert_eq!(classify(1), Some(Max));
        assert_eq!(classify(2), Some(Marker));
        assert_eq!(classify(3), Some(Marker));
        assert_eq!(classify(0), None);
        assert_eq!(classify(4), None);
    }

    #[test]
    fn note_bytes_round_trip_canonically() {
        // Entries sorted ascending, datasz 4, 4-byte padding per entry.
        let merged = m(&[(PROP_X86_ISA_1_NEEDED, 0x1), (PROP_X86_FEATURE_1_AND, 0x3)]);
        let note = build_property_note(&merged, false);
        let mut want = vec![
            4u8, 0, 0, 0, // namesz
            32, 0, 0, 0, // descsz = 2 * 16
            5, 0, 0, 0, // NT_GNU_PROPERTY_TYPE_0
            b'G', b'N', b'U', 0,
        ];
        want.extend_from_slice(&PROP_X86_FEATURE_1_AND.to_le_bytes());
        want.extend_from_slice(&4u32.to_le_bytes());
        want.extend_from_slice(&3u32.to_le_bytes());
        want.extend_from_slice(&[0u8; 4]);
        want.extend_from_slice(&PROP_X86_ISA_1_NEEDED.to_le_bytes());
        want.extend_from_slice(&4u32.to_le_bytes());
        want.extend_from_slice(&1u32.to_le_bytes());
        want.extend_from_slice(&[0u8; 4]);
        assert_eq!(note, want);
        // And the parser reads back exactly what the builder wrote.
        let carrier = synthetic_property_object(&note, false);
        assert_eq!(parse_property_note(&carrier, false), Some(merged));
    }

    #[test]
    fn objects_past_real_inputs_neither_veto_nor_carry() {
        // Two real inputs carry IBT|SHSTK; a linker-synthesized object
        // without the note (build-id, string-merge pool, ...) follows them.
        let note = build_property_note(&m(&[(PROP_X86_FEATURE_1_AND, 0x3)]), false);
        let mut objects = vec![
            synthetic_property_object(&note, false),
            synthetic_property_object(&note, false),
            super::super::build_id::synthetic_note_object(),
        ];
        let carrier = merge_property_into_objects(&mut objects, 2, &flags(), false).expect("merge");
        assert!(carrier.is_none(), "a real input carries the merged note");
        assert_eq!(
            parse_property_note(&objects[0], false),
            Some(m(&[(PROP_X86_FEATURE_1_AND, 0x3)]))
        );
        assert_eq!(
            parse_property_note(&objects[1], false),
            None,
            "duplicate cleared"
        );
        // The same object counted as a real input vetoes the AND type: the
        // boundary, not the object's contents, decides participation.
        let mut objects = vec![
            synthetic_property_object(&note, false),
            synthetic_property_object(&note, false),
            super::super::build_id::synthetic_note_object(),
        ];
        let carrier = merge_property_into_objects(&mut objects, 3, &flags(), false).expect("merge");
        assert!(carrier.is_none());
        assert!(
            objects
                .iter()
                .all(|o| parse_property_note(o, false).is_none())
        );
        // A boundary past the object list is an internal error, not a panic.
        assert!(merge_property_into_objects(&mut objects, 4, &flags(), false).is_err());
    }

    #[test]
    fn note_bytes_round_trip_32bit() {
        // 32-bit notes pack each entry into 12 bytes (no padding); the
        // parser must use the 4-stride or every entry after the first
        // misaligns.  Two entries pin the stride, not just the first.
        let merged = m(&[(PROP_X86_ISA_1_NEEDED, 0x1), (PROP_X86_FEATURE_1_AND, 0x3)]);
        let note = build_property_note(&merged, true);
        let mut want = vec![
            4u8, 0, 0, 0, // namesz
            24, 0, 0, 0, // descsz = 2 * 12
            5, 0, 0, 0, // NT_GNU_PROPERTY_TYPE_0
            b'G', b'N', b'U', 0,
        ];
        want.extend_from_slice(&PROP_X86_FEATURE_1_AND.to_le_bytes());
        want.extend_from_slice(&4u32.to_le_bytes());
        want.extend_from_slice(&3u32.to_le_bytes());
        want.extend_from_slice(&PROP_X86_ISA_1_NEEDED.to_le_bytes());
        want.extend_from_slice(&4u32.to_le_bytes());
        want.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(note, want);
        let carrier = synthetic_property_object(&note, true);
        assert_eq!(carrier.sections[1].addralign, 4);
        assert_eq!(parse_property_note(&carrier, true), Some(merged));
    }
}
