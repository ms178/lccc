//! GNU ld-exact `SHF_MERGE` section merging (binutils 2.47 `bfd/merge.c`).
//!
//! For the inputs of one output section, in output order, [`merge`]
//! reproduces what bfd does to every mergeable input:
//!
//! * eligibility (`_bfd_add_merge_section`): non-empty, `entsize` > 0 and
//!   dividing the size, no relocations of its own, and an alignment the
//!   entity size is compatible with (a string character size below the
//!   alignment must be a power of 2; otherwise the entity size must be a
//!   multiple of the alignment);
//! * grouping: inputs with equal `SHF_STRINGS`, `entsize` and alignment
//!   merge into one pool whose bytes live in the group's first input (the
//!   representative); the other members are excluded from layout;
//! * `record_section`: entities (strings, including their terminator, or
//!   fixed-size constants) are interned in first-seen order, each remembering
//!   the largest alignment it was found at (the lowest set bit of its input
//!   offset, capped at the section alignment); a string missing its final
//!   terminator gets one;
//! * `merge_strings`: tail merging -- an entity that is a suffix of a longer
//!   one with at least its alignment, at an offset its alignment allows,
//!   becomes a reference into it (`strrevcmp`/`strrevcmp_align` order);
//! * layout and `sec_merge_emit`: survivors in first-seen order, each
//!   zero-padded to its alignment, the pool padded to the section alignment
//!   when every member's size was a multiple of it;
//! * `_bfd_merged_section_offset`: input offsets map to the entity's pool
//!   offset plus the distance into the entity; offsets at or past the end of
//!   an input map to the end of the pool.
//!
//! Callers apply the result: a relocation against a section symbol resolves
//! `map(addend)`, one against any other symbol `map(value) + addend`
//! (`_bfd_elf_rela_local_sym` / `_bfd_elf_link_sec_merge_syms`).

use crate::common::fx_hash::FxHashMap;
use std::cmp::Ordering;
use std::rc::Rc;

/// One input of the output section being merged.
#[derive(Clone, Copy, Debug)]
pub struct MergeInput<'a> {
    pub data: &'a [u8],
    /// `SHF_MERGE` is set.
    pub merge: bool,
    /// `SHF_STRINGS` is set.
    pub strings: bool,
    pub entsize: u64,
    pub addralign: u64,
    /// The input carries relocations of its own (bfd's `SEC_RELOC`).
    pub has_relocs: bool,
}

/// Input offset -> pool offset of one merged input.
#[derive(Debug)]
pub struct OffsetMap {
    /// (input offset of an entity, its pool offset), sorted by input offset.
    entities: Vec<(u64, u64)>,
    rawsize: u64,
    pool_size: u64,
}

impl OffsetMap {
    /// `_bfd_merged_section_offset`.
    pub fn map(&self, offset: u64) -> u64 {
        if offset >= self.rawsize {
            return self.pool_size;
        }
        let i = self.entities.partition_point(|&(o, _)| o <= offset);
        let (o, idx) = self.entities[i - 1];
        idx + (offset - o)
    }
}

/// What one input becomes.
#[derive(Debug)]
pub enum Placement {
    /// Not merged: laid out as is.
    Raw,
    /// A merged input. `pool` (the merged bytes) is present on the group's
    /// representative, which is laid out in place with the pool as its
    /// contents; the other members are excluded from layout and resolve
    /// through the representative `repr` (an index into the inputs).
    Merged {
        repr: usize,
        pool: Option<Vec<u8>>,
        map: Rc<OffsetMap>,
    },
}

struct Entity {
    member: usize,
    start: usize,
    len: usize,
    alignment: u64,
    /// Tail-merged into this entity.
    suffix_of: Option<usize>,
    index: u64,
}

/// `_bfd_add_merge_section`'s eligibility test.
fn eligible(i: &MergeInput) -> bool {
    if !i.merge || i.data.is_empty() || i.entsize == 0 || i.has_relocs {
        return false;
    }
    let size = u64::try_from(i.data.len()).unwrap_or(u64::MAX);
    if size % i.entsize != 0 || u32::try_from(size).is_err() {
        return false;
    }
    let align = i.addralign.max(1);
    if !align.is_power_of_two() || align > u64::from(u32::MAX / 2 + 1) {
        return false;
    }
    let e = i.entsize;
    !((e < align && (!e.is_power_of_two() || !i.strings)) || (e > align && e & (align - 1) != 0))
}

/// `strrevcmp` (and `strrevcmp_align` when `align` is given), on entity
/// bytes without their terminator.
fn strrevcmp(a: &[u8], b: &[u8], align: Option<u64>) -> Ordering {
    if let Some(al) = align {
        let tail = |l: usize| u64::try_from(l).unwrap_or(0) & (al - 1);
        match tail(a.len()).cmp(&tail(b.len())) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .map(|(x, y)| x.cmp(y))
        .find(|o| o.is_ne())
        .unwrap_or_else(|| a.len().cmp(&b.len()))
}

/// Merge the mergeable inputs of one output section (see module docs).
pub fn merge(inputs: &[MergeInput]) -> Vec<Placement> {
    let mut placements: Vec<Placement> = inputs.iter().map(|_| Placement::Raw).collect();
    // Groups in first-member order: (strings, entsize, alignment) -> members.
    let mut groups: Vec<((bool, u64, u64), Vec<usize>)> = Vec::new();
    for (k, i) in inputs.iter().enumerate() {
        if !eligible(i) {
            continue;
        }
        let key = (i.strings, i.entsize, i.addralign.max(1));
        match groups.iter_mut().find(|(g, _)| *g == key) {
            Some((_, members)) => members.push(k),
            None => groups.push((key, vec![k])),
        }
    }
    for ((strings, entsize, align), members) in groups {
        merge_group(inputs, &members, strings, entsize, align, &mut placements);
    }
    placements
}

fn merge_group(
    inputs: &[MergeInput],
    members: &[usize],
    strings: bool,
    entsize: u64,
    align: u64,
    placements: &mut [Placement],
) {
    let es = usize::try_from(entsize).unwrap_or(usize::MAX);
    // Every member's bytes, string members with an extra zero entity
    // appended (`record_section`).
    let bufs: Vec<Vec<u8>> = members
        .iter()
        .map(|&k| {
            let mut v = inputs[k].data.to_vec();
            if strings {
                v.resize(v.len() + es, 0);
            }
            v
        })
        .collect();
    let mut entities: Vec<Entity> = Vec::new();
    let mut table: FxHashMap<(usize, usize), Vec<usize>> = FxHashMap::default();
    let mut offsets: Vec<Vec<(u64, usize)>> = Vec::with_capacity(members.len());
    for (m, &k) in members.iter().enumerate() {
        let buf = &bufs[m];
        let size = inputs[k].data.len();
        let mut recs = Vec::new();
        let mut p = 0usize;
        while p < size {
            let len = if strings {
                let mut n = 0;
                while buf[p + n..p + n + es].iter().any(|&b| b != 0) {
                    n += es;
                }
                n + es
            } else {
                es
            };
            let ofs = u64::try_from(p).unwrap_or(u64::MAX);
            let low = ofs.isolate_lowest_one();
            let eltalign = if low == 0 || low > align { align } else { low };
            let bytes = &buf[p..p + len];
            let hash = bytes
                .iter()
                .fold(len, |h, &b| h.wrapping_mul(31).wrapping_add(usize::from(b)));
            let bucket = table.entry((hash, len)).or_default();
            let found = bucket
                .iter()
                .copied()
                .find(|&e| entity_bytes(&bufs, &entities[e]) == bytes);
            let e = match found {
                Some(e) => {
                    entities[e].alignment = entities[e].alignment.max(eltalign);
                    e
                }
                None => {
                    let e = entities.len();
                    entities.push(Entity {
                        member: m,
                        start: p,
                        len,
                        alignment: eltalign,
                        suffix_of: None,
                        index: 0,
                    });
                    bucket.push(e);
                    e
                }
            };
            recs.push((ofs, e));
            p += len;
        }
        offsets.push(recs);
    }
    let pad_output = members
        .iter()
        .all(|&k| u64::try_from(inputs[k].data.len()).unwrap_or(1) % align == 0);
    let bytes_of = |e: &Entity| entity_bytes(&bufs, e);

    if strings && !entities.is_empty() {
        // `merge_strings`.
        let first = entities[0].alignment;
        let uniform = entities.iter().all(|e| e.alignment == first);
        let cmp_align = (uniform && first > entsize).then_some(first);
        let mut order: Vec<usize> = (0..entities.len()).collect();
        order.sort_by(|&a, &b| {
            let (ea, eb) = (&entities[a], &entities[b]);
            strrevcmp(
                &bytes_of(ea)[..ea.len - es],
                &bytes_of(eb)[..eb.len - es],
                cmp_align,
            )
        });
        let mut e = order[order.len() - 1];
        for &c in order[..order.len() - 1].iter().rev() {
            let (ee, ce) = (&entities[e], &entities[c]);
            let is_suffix = ee.len > ce.len && bytes_of(ee)[ee.len - ce.len..] == *bytes_of(ce);
            let delta = u64::try_from(ee.len - ce.len.min(ee.len)).unwrap_or(1);
            if ee.alignment >= ce.alignment && delta & (ce.alignment - 1) == 0 && is_suffix {
                entities[c].suffix_of = Some(e);
            } else {
                e = c;
            }
        }
    }
    let mut size = 0u64;
    for en in entities.iter_mut().filter(|en| en.suffix_of.is_none()) {
        size = size.div_ceil(en.alignment) * en.alignment;
        en.index = size;
        size += u64::try_from(en.len).unwrap_or(0);
    }
    let tail_index: Vec<Option<u64>> = entities
        .iter()
        .map(|en| {
            let s = &entities[en.suffix_of?];
            Some(s.index + u64::try_from(s.len - en.len).unwrap_or(0))
        })
        .collect();
    for (en, idx) in entities.iter_mut().zip(tail_index) {
        if let Some(idx) = idx {
            en.index = idx;
        }
    }
    if pad_output {
        size = size.div_ceil(align) * align;
    }
    // `sec_merge_emit`.
    let mut pool = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    for en in entities.iter().filter(|en| en.suffix_of.is_none()) {
        let at = usize::try_from(en.index).unwrap_or(usize::MAX);
        pool.resize(at, 0);
        pool.extend_from_slice(bytes_of(en));
    }
    pool.resize(usize::try_from(size).unwrap_or(usize::MAX), 0);

    let repr = members[0];
    for (m, &k) in members.iter().enumerate() {
        let map = OffsetMap {
            entities: offsets[m]
                .iter()
                .map(|&(o, e)| (o, entities[e].index))
                .collect(),
            rawsize: u64::try_from(inputs[k].data.len()).unwrap_or(u64::MAX),
            pool_size: size,
        };
        placements[k] = Placement::Merged {
            repr,
            pool: (m == 0).then(|| std::mem::take(&mut pool)),
            map: Rc::new(map),
        };
    }
}

fn entity_bytes<'a>(bufs: &'a [Vec<u8>], e: &Entity) -> &'a [u8] {
    &bufs[e.member][e.start..e.start + e.len]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(data: &[u8], entsize: u64, addralign: u64) -> MergeInput<'_> {
        MergeInput {
            data,
            merge: true,
            strings: true,
            entsize,
            addralign,
            has_relocs: false,
        }
    }

    fn pool(p: &Placement) -> &[u8] {
        match p {
            Placement::Merged { pool: Some(v), .. } => v,
            _ => panic!("not a representative: {p:?}"),
        }
    }

    fn map(p: &Placement, off: u64) -> u64 {
        match p {
            Placement::Merged { map, .. } => map.map(off),
            Placement::Raw => panic!("raw"),
        }
    }

    #[test]
    fn dedups_first_seen_and_tail_merges() {
        let a = b"main.c\0/usr/src\0";
        let b = b"src\0util.c\0main.c\0";
        let p = merge(&[input(a, 1, 1), input(b, 1, 1)]);
        // "src\0" is a suffix of "/usr/src\0"; "main.c\0" dedups.
        assert_eq!(pool(&p[0]), b"main.c\0/usr/src\0util.c\0");
        assert_eq!(map(&p[1], 0), 12, "src -> tail of /usr/src");
        assert_eq!(map(&p[1], 4), 16);
        assert_eq!(map(&p[1], 11), 0);
        assert_eq!(map(&p[1], 13), 2, "inside an entity keeps the distance");
        assert_eq!(map(&p[1], 18), 23, "end of input -> end of pool");
        assert!(matches!(
            p[1],
            Placement::Merged {
                repr: 0,
                pool: None,
                ..
            }
        ));
    }

    #[test]
    fn appends_a_missing_terminator() {
        let p = merge(&[input(b"ab\0cd", 1, 1)]);
        assert_eq!(pool(&p[0]), b"ab\0cd\0");
    }

    #[test]
    fn ineligible_inputs_stay_raw() {
        let mut r = input(b"x\0", 1, 1);
        r.has_relocs = true;
        let mut odd = input(b"abc", 2, 1);
        odd.strings = false;
        let mut bad_align = input(b"ab\0\0", 3, 2);
        bad_align.strings = false;
        let p = merge(&[r, odd, bad_align, input(b"", 1, 1)]);
        assert!(p.iter().all(|p| matches!(p, Placement::Raw)));
    }

    #[test]
    fn groups_by_entsize_and_alignment() {
        let p = merge(&[
            input(b"a\0", 1, 1),
            input(b"a\0\0\0\0\0\0\0", 1, 8),
            input(b"a\0", 1, 1),
        ]);
        assert!(matches!(p[0], Placement::Merged { repr: 0, .. }));
        assert!(matches!(p[1], Placement::Merged { repr: 1, .. }));
        assert!(matches!(p[2], Placement::Merged { repr: 0, .. }));
        // Align-8 group: "a\0" at 0 (alignment 8) and "\0" entities at
        // 2..7 (alignments 2, 1, 4, 1, 2, 1 -> one "\0" entity, max 4),
        // which is a suffix of "a\0" at a distance its alignment forbids.
        assert_eq!(pool(&p[1]), b"a\0\0\0\0\0\0\0");
    }

    #[test]
    fn constants_merge_without_tail_merging() {
        let mut a = input(&[1, 0, 0, 0, 2, 0, 0, 0], 4, 4);
        a.strings = false;
        let mut b = input(&[2, 0, 0, 0, 0, 0, 0, 0], 4, 4);
        b.strings = false;
        let p = merge(&[a, b]);
        assert_eq!(pool(&p[0]), &[1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(map(&p[1], 0), 4);
        assert_eq!(map(&p[1], 4), 8);
    }

    #[test]
    fn wide_strings_use_entity_sized_terminators() {
        let a = [b'h', 0, b'i', 0, 0, 0];
        let b = [b'i', 0, 0, 0];
        let p = merge(&[input(&a, 2, 2), input(&b, 2, 2)]);
        assert_eq!(pool(&p[0]), &a);
        assert_eq!(map(&p[1], 0), 2, "u\"i\" is a 2-aligned suffix of u\"hi\"");
    }
}
