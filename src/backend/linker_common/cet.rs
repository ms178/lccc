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
//!   real input has the type, value is the OR of all inputs, dropped when 0.
//!
//! Types outside every class range (binutils aborts on those — "never should
//! happen") are treated conservatively instead: kept only when every real
//! input has the type, ORed, dropped when 0.  A linker must never abort on
//! input data.
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
//! "Every real input" excludes synthetic objects the linker itself adds
//! (build-id, the property carrier) — they must not veto an AND-class type.
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
//! Note format (all fields little-endian; 64-bit shown — 32-bit notes use
//! 4-alignment instead of 8, so each entry is 12 bytes with no padding):
//! ```text
//!   u32 namesz = 4
//!   u32 descsz = 16 * n_entries        (12 * n_entries on 32-bit)
//!   u32 type   = 5                     (NT_GNU_PROPERTY_TYPE_0)
//!   "GNU\0"
//!   n_entries * ( u32 type, u32 datasz = 4, u32 value, 4 bytes of zeros )
//! ```
//! with the entries sorted by type ascending, as in the input objects.
//! (The `datasz` field is part of the property-entry ABI, per binutils
//! `readelf.c::print_gnu_property_note` — it is 4 for every x86 feature.)

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

/// Class ranges from binutils 2.47 `include/elf/common.h`
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
/// range, exactly as in `_bfd_x86_elf_merge_gnu_properties` — matching on
/// the five known constants instead silently mis-merges every other type in
/// the ranges.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PropertyClass {
    And,
    Or,
    OrAnd,
}

fn classify(t: u32) -> PropertyClass {
    if t == PROP_X86_COMPAT_ISA_1_NEEDED || (OR_LO..=OR_HI).contains(&t) {
        PropertyClass::Or
    } else if t == PROP_X86_COMPAT_ISA_1_USED || (OR_AND_LO..=OR_AND_HI).contains(&t) {
        PropertyClass::OrAnd
    } else if (AND_LO..=AND_HI).contains(&t) {
        PropertyClass::And
    } else {
        // Out-of-range unknown type: binutils aborts ("never should
        // happen").  A linker must not abort on input data; the OR-AND rule
        // (unanimous, ORed, dropped when 0) is the conservative choice.
        PropertyClass::OrAnd
    }
}

/// Per-type merge of the AND / OR / OR-AND property classes.
///
/// `inputs` is one map per *real* input object (type → value); objects
/// without a property section contribute an empty map, which makes them
/// veto AND / OR-AND types.  Returns the merged set in ascending type order.
fn merge_properties(
    inputs: &[FxHashMap<u32, u32>],
    flags: &PropertyLinkFlags,
) -> FxHashMap<u32, u32> {
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
    let mut types: Vec<u32> = all_types.into_iter().collect();
    types.sort_unstable();

    let mut merged: FxHashMap<u32, u32> = FxHashMap::default();
    for &t in &types {
        // `all` is vacuously true for an empty input list, but "every
        // input has the type" is meaningless when there are no inputs:
        // the AND identity (all-ones) must only apply when at least one
        // map actually carries the type.  Without the guard, `-z shstk`
        // with no inputs produced FEATURE_1_AND = 0xffff_ffff instead of
        // just the requested bit.
        let all_have = !inputs.is_empty() && inputs.iter().all(|m| m.contains_key(&t));
        let or_of_inputs: u32 = inputs
            .iter()
            .filter_map(|m| m.get(&t))
            .fold(0, |a, &b| a | b);
        match classify(t) {
            PropertyClass::And => {
                // AND of all inputs when every input has the type,
                // otherwise a 0 base; then the -z feature bits (only
                // FEATURE_1_AND has command-line bits).
                let mut value = if all_have {
                    inputs.iter().map(|m| m[&t]).fold(u32::MAX, |a, b| a & b)
                } else {
                    0
                };
                if t == PROP_X86_FEATURE_1_AND {
                    value |= flags.feature1_bits();
                }
                if value != 0 {
                    merged.insert(t, value);
                }
            }
            PropertyClass::Or => {
                // OR of whoever has the type, plus the -z ISA bit for
                // ISA_1_NEEDED.  An all-zero result is REMOVED (binutils
                // drops OR-class properties whose bits are all empty) —
                // keeping a value-0 entry would diverge from GNU ld.  (A
                // seeded ISA_1_NEEDED always carries the isa bit, so the
                // create-from-command-line case still inserts.)
                let mut value = or_of_inputs;
                if t == PROP_X86_ISA_1_NEEDED {
                    value |= flags.isa_bit();
                }
                if value != 0 {
                    merged.insert(t, value);
                }
            }
            PropertyClass::OrAnd => {
                // Kept only when every input has the type; ORed; a 0
                // result is dropped.  (Binutils keeps the entry with value
                // 0 in the both-present OR step and removes it right after
                // — observably identical to dropping it here.)
                if all_have && or_of_inputs != 0 {
                    merged.insert(t, or_of_inputs);
                }
            }
        }
    }
    merged
}

/// Parse the property note of one object; `None` when the object has no
/// (or an empty / malformed) `.note.gnu.property` section.
///
/// `is_32` selects the ELF-class entry stride: binutils pads each
/// property's data to 8 bytes on 64-bit targets and 4 on 32-bit ones
/// (`elf-properties.c`: `align_size = elfclass == ELFCLASS64 ? 8 : 4`;
/// `ptr += (datasz + (align_size - 1)) & ~(align_size - 1)`), so a CET
/// entry occupies 16 bytes in a 64-bit note and 12 in a 32-bit note.
/// Parsing with the wrong stride misaligns every entry after the first.
fn parse_property_note(obj: &Elf64Object, is_32: bool) -> Option<FxHashMap<u32, u32>> {
    let si = obj
        .sections
        .iter()
        .position(|s| s.name == PROPERTY_SECTION)?;
    let data = obj.section_data[si].as_slice();
    // Nhdr (12 bytes) + "GNU\0" (4).
    if data.len() < 16 {
        return None;
    }
    let namesz = u32::from_le_bytes(data[0..4].try_into().ok()?) as usize;
    let descsz = u32::from_le_bytes(data[4..8].try_into().ok()?) as usize;
    let ntype = u32::from_le_bytes(data[8..12].try_into().ok()?);
    if ntype != NT_GNU_PROPERTY || namesz != 4 || data.len() < 12 + 4 + descsz {
        return None;
    }
    let desc = &data[16..16 + descsz];
    if desc.is_empty() {
        // A note with no property entries is equivalent to none.
        return None;
    }
    // Entry stride: 8 header bytes + data padded to the class alignment
    // (8 on 64-bit, 4 on 32-bit — see the doc comment above).  A 4-byte
    // x86 bitmask therefore steps 16 bytes in a 64-bit note and 12 in a
    // 32-bit note.  (Parenthesised on purpose: `+` binds tighter than
    // `&`, so the bare form computes the same value — but only by
    // precedence luck.)
    let align = if is_32 { 4usize } else { 8usize };
    let mut map: FxHashMap<u32, u32> = FxHashMap::default();
    let mut o = 0usize;
    while o + 8 <= descsz {
        let t = u32::from_le_bytes(desc[o..o + 4].try_into().ok()?);
        let datasz = u32::from_le_bytes(desc[o + 4..o + 8].try_into().ok()?);
        let data_off = o + 8;
        let step = 8usize + (((datasz as usize) + (align - 1)) & !(align - 1));
        if data_off + datasz as usize > descsz || o + step > descsz {
            // Corrupted descriptor; stop rather than read out of bounds.
            break;
        }
        // x86 feature properties carry a 4-byte bitmask.
        if datasz == 4 {
            let v = u32::from_le_bytes(desc[data_off..data_off + 4].try_into().ok()?);
            map.insert(t, v);
        }
        o += step;
    }
    Some(map)
}

/// Parse the property notes of every *real* input.
fn per_input_maps(
    objects: &[Elf64Object],
    synthetic: &FxHashSet<usize>,
    is_32: bool,
) -> Result<Vec<FxHashMap<u32, u32>>, String> {
    let mut inputs = Vec::with_capacity(objects.len());
    for (i, obj) in objects.iter().enumerate() {
        if synthetic.contains(&i) {
            continue;
        }
        inputs.push(parse_property_note(obj, is_32).unwrap_or_default());
    }
    Ok(inputs)
}

/// Build the merged note bytes in the canonical layout (entries sorted by
/// type ascending): 16-byte entries on 64-bit targets, 12-byte entries on
/// 32-bit targets (see [`parse_property_note`]).
fn build_property_note(merged: &FxHashMap<u32, u32>, is_32: bool) -> Vec<u8> {
    let mut types: Vec<u32> = merged.keys().copied().collect();
    types.sort_unstable();
    let entry_len = if is_32 { 12 } else { 16 };
    let descsz = types.len() * entry_len;
    let mut out = Vec::with_capacity(12 + 4 + descsz);
    out.extend_from_slice(&4u32.to_le_bytes()); // namesz
    out.extend_from_slice(&(descsz as u32).to_le_bytes());
    out.extend_from_slice(&NT_GNU_PROPERTY.to_le_bytes());
    out.extend_from_slice(b"GNU\0");
    for &t in &types {
        out.extend_from_slice(&t.to_le_bytes());
        out.extend_from_slice(&4u32.to_le_bytes()); // datasz
        out.extend_from_slice(&merged[&t].to_le_bytes());
        if !is_32 {
            out.extend_from_slice(&[0u8; 4]); // padding to the 8-alignment
        }
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
/// `synthetic` marks objects whose note must not take part in the merge
/// (build-id and other linker-generated objects); update it *after*
/// appending the returned carrier so it does not veto the merge.
/// `is_32` is the link's ELF class: property entries are 16 bytes on
/// 64-bit targets and 12 on 32-bit ones (see [`parse_property_note`]).
pub fn merge_property_into_objects(
    objects: &mut [Elf64Object],
    synthetic: &FxHashSet<usize>,
    flags: &PropertyLinkFlags,
    is_32: bool,
) -> Result<Option<Elf64Object>, String> {
    let inputs = per_input_maps(objects, synthetic, is_32)?;
    let merged = merge_properties(&inputs, flags);
    let note = if merged.is_empty() {
        None
    } else {
        Some(build_property_note(&merged, is_32))
    };

    let first_carrier = objects
        .iter()
        .enumerate()
        .find(|(i, o)| {
            !synthetic.contains(i) && o.sections.iter().any(|s| s.name == PROPERTY_SECTION)
        })
        .map(|(i, _)| i);

    match (note.as_ref(), first_carrier) {
        (Some(n), Some(i)) => {
            let o = &mut objects[i];
            let si = o
                .sections
                .iter()
                .position(|s| s.name == PROPERTY_SECTION)
                .unwrap();
            o.section_data[si] = SectionData::owned(n.clone());
            o.sections[si].size = n.len() as u64;
            o.sections[si].addralign = o.sections[si].addralign.max(if is_32 { 4 } else { 8 });
            for (j, other) in objects.iter_mut().enumerate() {
                if j != i && !synthetic.contains(&j) {
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
            for (j, other) in objects.iter_mut().enumerate() {
                if !synthetic.contains(&j) {
                    clear_property_section(other);
                }
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: &[(u32, u32)]) -> FxHashMap<u32, u32> {
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
        );
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&0x1));
        // ... and nothing (base 0, dropped) when any input lacks it.
        let got = merge_properties(&[m(&[(PROP_X86_FEATURE_1_AND, 0x3)]), m(&[])], &flags());
        assert!(!got.contains_key(&PROP_X86_FEATURE_1_AND));
    }

    #[test]
    fn and_class_z_bits_rescue_and_create() {
        // `-z ibt` rescues a vetoed AND type (IBT only, not the input bits).
        let f = PropertyLinkFlags {
            ibt: true,
            ..flags()
        };
        let got = merge_properties(&[m(&[(PROP_X86_FEATURE_1_AND, 0x2)]), m(&[])], &f);
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&FEATURE_1_IBT));
        // `-z shstk` creates the property with no inputs at all.
        let f = PropertyLinkFlags {
            shstk: true,
            ..flags()
        };
        let got = merge_properties(&[], &f);
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&FEATURE_1_SHSTK));
        // `-z lam-u48` sets both LAM bits, like binutils.
        let f = PropertyLinkFlags {
            lam_u48: true,
            ..flags()
        };
        let got = merge_properties(&[], &f);
        assert_eq!(got.get(&PROP_X86_FEATURE_1_AND), Some(&0xc));
    }

    #[test]
    fn and_rule_applies_to_the_whole_and_range() {
        // A hypothetical second AND-range type must be ANDed, not ORed:
        // classification is by range, not by constant.
        let t = 0xc000_0003u32;
        assert_eq!(classify(t), PropertyClass::And);
        let got = merge_properties(&[m(&[(t, 0b110)]), m(&[(t, 0b101)])], &flags());
        assert_eq!(got.get(&t), Some(&0b100));
        let got = merge_properties(&[m(&[(t, 0b110)]), m(&[])], &flags());
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
        );
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&0x5));
        // ... but an all-zero OR result is REMOVED (binutils drops
        // OR-class properties whose bits are all empty).
        let got = merge_properties(&[m(&[(PROP_X86_ISA_1_NEEDED, 0x0)])], &flags());
        assert!(!got.contains_key(&PROP_X86_ISA_1_NEEDED));
        // FEATURE_1_NEEDED shares the OR rule.
        let t = 0xc000_8001u32;
        let got = merge_properties(&[m(&[(t, 0x1)]), m(&[])], &flags());
        assert_eq!(got.get(&t), Some(&0x1));
    }

    #[test]
    fn compat_needed_is_or_class() {
        // 0xc0000001 sits below the ranges but binutils merges it with OR
        // semantics: kept when ANY input has it.
        assert_eq!(classify(PROP_X86_COMPAT_ISA_1_NEEDED), PropertyClass::Or);
        let got = merge_properties(
            &[m(&[(PROP_X86_COMPAT_ISA_1_NEEDED, 0x2)]), m(&[])],
            &flags(),
        );
        assert_eq!(got.get(&PROP_X86_COMPAT_ISA_1_NEEDED), Some(&0x2));
    }

    #[test]
    fn or_and_class_requires_unanimity() {
        let t = PROP_X86_ISA_1_USED;
        let got = merge_properties(&[m(&[(t, 0x3)]), m(&[(t, 0x5)])], &flags());
        assert_eq!(got.get(&t), Some(&0x7));
        // One input without the type vetoes it ...
        let got = merge_properties(&[m(&[(t, 0x3)]), m(&[])], &flags());
        assert!(!got.contains_key(&t));
        // ... as does an all-zero OR.
        let got = merge_properties(&[m(&[(t, 0x0)]), m(&[(t, 0x0)])], &flags());
        assert!(!got.contains_key(&t));
    }

    #[test]
    fn compat_used_is_or_and_class() {
        assert_eq!(classify(PROP_X86_COMPAT_ISA_1_USED), PropertyClass::OrAnd);
        let got = merge_properties(
            &[
                m(&[(PROP_X86_COMPAT_ISA_1_USED, 0x1)]),
                m(&[(PROP_X86_COMPAT_ISA_1_USED, 0x2)]),
            ],
            &flags(),
        );
        assert_eq!(got.get(&PROP_X86_COMPAT_ISA_1_USED), Some(&0x3));
        let got = merge_properties(&[m(&[(PROP_X86_COMPAT_ISA_1_USED, 0x1)]), m(&[])], &flags());
        assert!(!got.contains_key(&PROP_X86_COMPAT_ISA_1_USED));
    }

    #[test]
    fn isa_level_injects_and_creates() {
        // `-z x86-64-v3` with no inputs creates ISA_1_NEEDED = V3.
        let f = PropertyLinkFlags {
            isa_level: 3,
            ..flags()
        };
        let got = merge_properties(&[], &f);
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&ISA_1_V3));
        // With inputs the level bit is ORed in (baseline | v2).
        let f = PropertyLinkFlags {
            isa_level: 2,
            ..flags()
        };
        let got = merge_properties(&[m(&[(PROP_X86_ISA_1_NEEDED, 0x1)])], &f);
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&0x3));
        // Baseline ORs the baseline bit instead of aborting the merge
        // (binutils 2.47 BFD-internal-errors here — missing `case 1:`).
        let f = PropertyLinkFlags {
            isa_level: 1,
            ..flags()
        };
        let got = merge_properties(&[m(&[(PROP_X86_ISA_1_NEEDED, 0x2)])], &f);
        assert_eq!(got.get(&PROP_X86_ISA_1_NEEDED), Some(&0x3));
        // Levels outside 1..=4 inject nothing (the CLI layer rejects them;
        // the merge must still be total).
        let f = PropertyLinkFlags {
            isa_level: 9,
            ..flags()
        };
        let got = merge_properties(&[], &f);
        assert!(got.is_empty());
    }

    #[test]
    fn out_of_range_types_are_conservative() {
        // binutils aborts on these; lccc keeps them iff unanimous.
        let t = 0x1234_5678u32;
        assert_eq!(classify(t), PropertyClass::OrAnd);
        let got = merge_properties(&[m(&[(t, 0x9)]), m(&[(t, 0x9)])], &flags());
        assert_eq!(got.get(&t), Some(&0x9));
        let got = merge_properties(&[m(&[(t, 0x9)]), m(&[])], &flags());
        assert!(!got.contains_key(&t));
    }

    #[test]
    fn class_boundaries_match_binutils_2_47_common_h() {
        assert_eq!(classify(0xc000_0002), PropertyClass::And);
        assert_eq!(classify(0xc000_7fff), PropertyClass::And);
        assert_eq!(classify(0xc000_8000), PropertyClass::Or);
        assert_eq!(classify(0xc000_ffff), PropertyClass::Or);
        assert_eq!(classify(0xc001_0000), PropertyClass::OrAnd);
        assert_eq!(classify(0xc001_7fff), PropertyClass::OrAnd);
        assert_eq!(classify(0xc000_8001), PropertyClass::Or);
        assert_eq!(classify(0xc001_0001), PropertyClass::OrAnd);
        assert_eq!(classify(0x0000_0000), PropertyClass::OrAnd);
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
