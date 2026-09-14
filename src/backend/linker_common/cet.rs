//! GNU x86 property note (`.note.gnu.property`) merging.
//!
//! The kernel, glibc and any CET-enabled object carry 64-bit properties in a
//! `.note.gnu.property` note (type `NT_GNU_PROPERTY_TYPE_0` = 5).  At link
//! time GNU ld folds the per-object property sets into one output note using
//! class-dependent rules (see binutils `bfd/elf-properties.c`,
//! `_bfd_x86_elf_merge_gnu_properties`, verified against binutils 2.44 and
//! lld):
//!
//! * **AND class** — types `0xc0000002..=0xc0007fff`
//!   (`GNU_PROPERTY_X86_FEATURE_1_AND` = `0xc0000002`): the value is the
//!   AND of every real input that has the type; if any real input lacks the
//!   type the base value is 0.  The `-z ibt` / `-z shstk` / `-z lam-u48` /
//!   `-z lam-u57` bits are ORed in afterwards (they can create the property
//!   even when no input had it).  A result of 0 drops the type.
//! * **OR class** — types `0xc0008000..=0xc000ffff`
//!   (`GNU_PROPERTY_X86_FEATURE_1_NEEDED` = `0xc0008001`,
//!   `GNU_PROPERTY_X86_ISA_1_NEEDED` = `0xc0008002`): values are ORed
//!   across all inputs that have the type, and the type is kept when any
//!   input has it (the value may be 0: that is the "x86-64-baseline" note
//!   glibc's CRT carries).  The ISA level is *not* a linker option — GNU ld
//!   has no `-march`/`--isa-level`; the ISA bits arrive exclusively via the
//!   input objects' notes (set by the compiler driver at compile time).
//! * **OR-AND class** — types `0xc0010000..=0xc0017fff`
//!   (`GNU_PROPERTY_X86_FEATURE_2_USED` = `0xc0010001`,
//!   `GNU_PROPERTY_X86_ISA_1_USED` = `0xc0010002`): kept only when every
//!   real input has the type, value is the OR of all inputs.
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
//! Note format (all fields little-endian):
//! ```text
//!   u32 namesz = 4
//!   u32 descsz = 16 * n_entries
//!   u32 type   = 5                     (NT_GNU_PROPERTY_TYPE_0)
//!   "GNU\0"
//!   n_entries * ( u32 type, u32 datasz = 4, u32 value, 4 bytes of zeros )
//! ```
//! with the entries sorted by type ascending, as in the input objects.
//! (The `datasz` field is part of the property-entry ABI, per binutils
//! `readelf.c::print_gnu_property_note` — it is 4 for every x86 feature.)

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::types::{Elf64Object, Elf64Section};
use super::SectionData;

/// Section name carrying the 64-bit property note.
pub const PROPERTY_SECTION: &str = ".note.gnu.property";

/// `NT_GNU_PROPERTY_TYPE_0` — the note type gcc/bfd use for the x86
/// property note on 64-bit targets.
const NT_GNU_PROPERTY: u32 = 5;

/// `GNU_PROPERTY_X86_FEATURE_1_AND`.
const PROP_X86_FEATURE_1_AND: u32 = 0xc000_0002;
/// `GNU_PROPERTY_X86_FEATURE_1_NEEDED`.
const PROP_X86_FEATURE_1_NEEDED: u32 = 0xc000_8001;
/// `GNU_PROPERTY_X86_ISA_1_NEEDED`.
const PROP_X86_ISA_1_NEEDED: u32 = 0xc000_8002;
/// `GNU_PROPERTY_X86_FEATURE_2_USED`.
const PROP_X86_FEATURE_2_USED: u32 = 0xc001_0001;
/// `GNU_PROPERTY_X86_ISA_1_USED`.
const PROP_X86_ISA_1_USED: u32 = 0xc001_0002;

/// `-z ibt` bit in `GNU_PROPERTY_X86_FEATURE_1_AND`.
const FEATURE_1_IBT: u32 = 0x1;
/// `-z shstk` bit.
const FEATURE_1_SHSTK: u32 = 0x2;
/// `-z lam-u48` bits (both LAM bits, matching GNU's `LZCNT_LAM` grouping).
const FEATURE_1_LAM_U48: u32 = 0x4 | 0x8;
/// `-z lam-u57` bit.
const FEATURE_1_LAM_U57: u32 = 0x8;

/// `-z` options that influence the property note.  (The ISA level is
/// deliberately absent: GNU ld has no command-line option for it — the
/// `GNU_PROPERTY_X86_ISA_1_NEEDED` bits come from the input objects' notes
/// only, set by the compiler driver at compile time.)
#[derive(Debug, Clone, Copy, Default)]
pub struct PropertyLinkFlags {
    pub ibt: bool,
    pub shstk: bool,
    pub lam_u48: bool,
    pub lam_u57: bool,
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
    let mut types: Vec<u32> = all_types.into_iter().collect();
    types.sort_unstable();

    let mut merged: FxHashMap<u32, u32> = FxHashMap::default();
    for &t in &types {
        let all_have = inputs.iter().all(|m| m.contains_key(&t));
        let mut value: u32 = 0;
        match t {
            PROP_X86_FEATURE_1_AND => {
                value = if all_have {
                    inputs
                        .iter()
                        .map(|m| m[&t])
                        .fold(u32::MAX, |a, b| a & b)
                } else {
                    0
                };
                // The -z option bits replace (do not depend on) the input
                // bits: even when some input lacks the type, `-z ibt`
                // yields IBT only.
                value |= flags.feature1_bits();
                if value != 0 {
                    merged.insert(t, value);
                }
            }
            PROP_X86_FEATURE_1_NEEDED | PROP_X86_ISA_1_NEEDED => {
                for m in inputs {
                    if let Some(&v) = m.get(&t) {
                        value |= v;
                    }
                }
                // OR-class types survive when any input carried them (the
                // value may legitimately be 0 — the baseline ISA note).
                if !inputs.iter().all(|m| !m.contains_key(&t)) {
                    merged.insert(t, value);
                }
            }
            PROP_X86_FEATURE_2_USED | PROP_X86_ISA_1_USED => {
                if all_have {
                    for m in inputs {
                        value |= m[&t];
                    }
                    if value != 0 {
                        merged.insert(t, value);
                    }
                }
            }
            // Unknown property types: keep when every input has the type,
            // value = OR (the same rule binutils applies to unclassified
            // types in the OR-AND range); drop otherwise.
            0xc001_0000..=0xc001_7fff => {
                if all_have {
                    for m in inputs {
                        value |= m[&t];
                    }
                    if value != 0 {
                        merged.insert(t, value);
                    }
                }
            }
            _ => {
                // Types outside the property ranges are not merged at all
                // (binutils copies them unchanged only when every input has
                // them; treat like the OR-AND rule).
                if all_have {
                    for m in inputs {
                        value |= m[&t];
                    }
                    if value != 0 {
                        merged.insert(t, value);
                    }
                }
            }
        }
    }
    merged
}

/// Parse the property note of one object; `None` when the object has no
/// (or an empty / malformed) `.note.gnu.property` section.
fn parse_property_note(obj: &Elf64Object) -> Option<FxHashMap<u32, u32>> {
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
    let mut map: FxHashMap<u32, u32> = FxHashMap::default();
    let mut o = 0usize;
    while o + 8 <= descsz {
        let t = u32::from_le_bytes(desc[o..o + 4].try_into().ok()?);
        let datasz = u32::from_le_bytes(desc[o + 4..o + 8].try_into().ok()?);
        let data_off = o + 8;
        let step = 8usize + ((datasz as usize) + 3) & !3;
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
) -> Result<Vec<FxHashMap<u32, u32>>, String> {
    let mut inputs = Vec::with_capacity(objects.len());
    for (i, obj) in objects.iter().enumerate() {
        if synthetic.contains(&i) {
            continue;
        }
        inputs.push(parse_property_note(obj).unwrap_or_default());
    }
    Ok(inputs)
}

/// Build the merged note bytes in the canonical layout (entries sorted by
/// type ascending).
fn build_property_note(merged: &FxHashMap<u32, u32>) -> Vec<u8> {
    let mut types: Vec<u32> = merged.keys().copied().collect();
    types.sort_unstable();
    let descsz = types.len() * 16;
    let mut out = Vec::with_capacity(12 + 4 + descsz);
    out.extend_from_slice(&4u32.to_le_bytes()); // namesz
    out.extend_from_slice(&(descsz as u32).to_le_bytes());
    out.extend_from_slice(&NT_GNU_PROPERTY.to_le_bytes());
    out.extend_from_slice(b"GNU\0");
    for &t in &types {
        out.extend_from_slice(&t.to_le_bytes());
        out.extend_from_slice(&4u32.to_le_bytes()); // datasz
        out.extend_from_slice(&merged[&t].to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // padding
    }
    out
}

/// A synthetic object carrying a single-section property note, mirroring
/// [`super::build_id::synthetic_note_object`]: section 0 is the NULL
/// placeholder, section 1 is the note.
fn synthetic_property_object(note: &[u8]) -> Elf64Object {
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
            addralign: 8,
            entsize: 0,
        },
    ];
    let section_data = vec![
        SectionData::empty(),
        SectionData::owned(note.to_vec()),
    ];
    Elf64Object {
        sections,
        symbols: Vec::new(),
        section_data,
        relocations: vec![Vec::new(); 2],
        source_name: "<property>".into(),
    }
}

fn clear_property_section(obj: &mut Elf64Object) {
    if let Some(si) = obj
        .sections
        .iter()
        .position(|s| s.name == PROPERTY_SECTION)
    {
        obj.section_data[si] = SectionData::empty();
        obj.sections[si].size = 0;
    }
}

/// Fold every real input's `.note.gnu.property` into a single note and
/// rewrite the *input objects* in place: the first real carrier keeps the
/// merged bytes, all other copies are cleared.  When no input carried the
/// section but `-z` / ISA options still require one, a synthetic carrier
/// object is returned for the caller to append (mirroring
/// [`super::build_id::synthetic_note_object`]) so the section merge emits
/// the note.
///
/// `synthetic` marks objects whose note must not take part in the merge
/// (build-id and other linker-generated objects); update it *after*
/// appending the returned carrier so it does not veto the merge.
pub fn merge_property_into_objects(
    objects: &mut [Elf64Object],
    synthetic: &FxHashSet<usize>,
    flags: &PropertyLinkFlags,
) -> Result<Option<Elf64Object>, String> {
    let inputs = per_input_maps(objects, synthetic)?;
    let merged = merge_properties(&inputs, flags);
    let note = if merged.is_empty() {
        None
    } else {
        Some(build_property_note(&merged))
    };

    let first_carrier = objects
        .iter()
        .enumerate()
        .find(|(i, o)| {
            !synthetic.contains(i)
                && o.sections.iter().any(|s| s.name == PROPERTY_SECTION)
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
            o.sections[si].addralign = o.sections[si].addralign.max(8);
            for (j, other) in objects.iter_mut().enumerate() {
                if j != i && !synthetic.contains(&j) {
                    clear_property_section(other);
                }
            }
            Ok(None)
        }
        (Some(n), None) => Ok(Some(synthetic_property_object(n))),
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
