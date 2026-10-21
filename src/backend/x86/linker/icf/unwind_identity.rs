//! Immutable unwind identity for text ICF. Incoming FDE edges are not visible
//! in text relocations. Normalize only the CIE's position-dependent backpointer;
//! compare the actual CFI, personality references, LSDA bytes and relocations.
//! This deliberately uses exact symbol identity, NOT evolving ICF classes, so
//! no hidden dependency can escape the refinement worklist. Unusual/malformed
//! graphs lose folding rather than acquire a guessed unwind program.
use super::*;
use std::hash::Hash;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Identity {
    #[default]
    None,
    Exact {
        cie: Blob,
        fde: Blob,
    },
    Opaque(SecId),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Blob {
    bytes: Vec<u8>,
    relocs: Vec<(u64, u32, i64, Target)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Target {
    SelfText(u64),
    Global(crate::backend::linker_common::SymStr),
    Absolute(u64),
    Local(SecId, u64),
    Lsda(u64, Box<Blob>),
}

impl Identity {
    pub(super) fn content_hash(&self, bytes: &[u8]) -> u64 {
        if matches!(self, Self::None) {
            return hash_bytes(bytes);
        }
        let mut h = FxHasher::default();
        bytes.hash(&mut h);
        self.hash(&mut h);
        h.finish()
    }
}

fn is_lsda(name: &str) -> bool {
    name == ".gcc_except_table" || name.starts_with(".gcc_except_table.")
}

fn relocation_width(kind: u32) -> Option<u64> {
    Some(match kind {
        R_X86_64_64 | R_X86_64_PC64 => 8,
        R_X86_64_PC32 | R_X86_64_32 | R_X86_64_32S | R_X86_64_PLT32 => 4,
        R_X86_64_PC16 => 2,
        R_X86_64_PC8 => 1,
        _ => return None,
    })
}

struct Reader<'a> {
    obj: &'a Elf64Object,
    oi: usize,
    relocs: FxHashMap<usize, Option<Vec<&'a Elf64Rela>>>,
    remaining: std::cell::Cell<usize>,
}

impl<'a> Reader<'a> {
    fn new(obj: &'a Elf64Object, oi: usize) -> Self {
        let relocs = obj
            .relocations
            .iter()
            .enumerate()
            .filter(|(si, _)| is_unwind_metadata_name(&obj.sections[*si].name))
            .map(|(si, rs)| {
                (
                    si,
                    (|| {
                        let mut sorted: Vec<_> =
                            rs.iter().filter(|r| r.rela_type != R_X86_64_NONE).collect();
                        sorted.sort_by_key(|r| r.offset);
                        let mut end = 0;
                        for r in &sorted {
                            let width = relocation_width(r.rela_type)?;
                            if r.offset < end {
                                return None;
                            }
                            end = r.offset.checked_add(width)?;
                            if end > obj.section_data[si].as_slice().len() as u64 {
                                return None;
                            }
                        }
                        Some(sorted)
                    })(),
                )
            })
            .collect();
        Self {
            obj,
            oi,
            relocs,
            remaining: std::cell::Cell::new(16 * 1024 * 1024),
        }
    }
    fn blob(
        &self,
        si: usize,
        range: std::ops::Range<usize>,
        own_text: usize,
        clear: Option<(usize, usize)>,
        lsda: bool,
    ) -> Option<Blob> {
        let (obj, oi) = (self.obj, self.oi);
        let (start, end) = (range.start, range.end);
        let data = obj.section_data.get(si)?.as_slice();
        let raw = data.get(start..end)?;
        // Bound duplicated CIE/whole-section LSDA storage on large or hostile
        // TUs. Budget exhaustion makes this object's text identities opaque.
        self.remaining
            .set(self.remaining.get().checked_sub(raw.len())?);
        let mut bytes = raw.to_vec();
        if let Some((at, size)) = clear {
            bytes
                .get_mut(at.checked_sub(start)?..at.checked_sub(start)?.checked_add(size)?)?
                .fill(0);
        }
        let mut relocs = Vec::new();
        let rs = self.relocs.get(&si)?.as_ref()?;
        let lo = rs.partition_point(|r| r.offset < start as u64);
        let hi = rs.partition_point(|r| r.offset < end as u64);
        for r in &rs[lo..hi] {
            if r.offset.checked_add(relocation_width(r.rela_type)?)? > end as u64 {
                return None;
            }
            if r.rela_type == R_X86_64_NONE {
                continue;
            }
            let sym = obj.symbols.get(r.sym_idx as usize)?;
            let target = if !sym.is_local() && !sym.name.is_empty() {
                // Binding by name, including preemption/weak resolution, must stay
                // binding by name. Never substitute a local data-content guess.
                Target::Global(sym.name.clone())
            } else if sym.shndx == SHN_ABS {
                Target::Absolute(sym.value)
            } else if sym.shndx == SHN_UNDEF || sym.shndx >= SHN_LORESERVE {
                return None;
            } else if sym.shndx as usize == own_text {
                Target::SelfText(sym.value)
            } else {
                let target_si = sym.shndx as usize;
                let target_sec = obj.sections.get(target_si)?;
                if is_lsda(&target_sec.name) {
                    // LSDA -> LSDA cycles/graphs are intentionally not traversed.
                    if lsda {
                        return None;
                    }
                    let size = obj.section_data.get(target_si)?.as_slice().len();
                    if sym.value > size as u64 {
                        return None;
                    }
                    Target::Lsda(
                        sym.value,
                        Box::new(self.blob(target_si, 0..size, own_text, None, true)?),
                    )
                } else {
                    Target::Local((oi, target_si), sym.value)
                }
            };
            relocs.push((r.offset - start as u64, r.rela_type, r.addend, target));
        }
        Some(Blob { bytes, relocs })
    }
}

pub(super) fn identities(objects: &[Elf64Object]) -> FxHashMap<SecId, Identity> {
    let mut out = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        if !obj
            .sections
            .iter()
            .any(|s| s.name == ".eh_frame" || s.name.starts_with(".eh_frame."))
        {
            continue;
        }
        let reader = Reader::new(obj, oi);
        let mut malformed = false;
        for (si, section) in obj.sections.iter().enumerate() {
            if section.name != ".eh_frame" && !section.name.starts_with(".eh_frame.") {
                continue;
            }
            let data = obj.section_data[si].as_slice();
            let records = crate::backend::linker_common::scan_eh_frame_records(data);
            let Some(relas) = reader.relocs.get(&si).and_then(Option::as_ref) else {
                malformed = true;
                continue;
            };
            for r in relas {
                let n = records.partition_point(|rec| rec.start as u64 <= r.offset);
                if n == 0
                    || r.offset >= records[n - 1].end as u64
                    || r.offset < (records[n - 1].id_offset + records[n - 1].id_size) as u64
                {
                    malformed = true;
                }
            }
            let mut previous = 0;
            for rec in &records {
                if data[previous..rec.start].iter().any(|b| *b != 0) {
                    malformed = true;
                }
                previous = rec.end;
            }
            if data[previous..].iter().any(|b| *b != 0) {
                malformed = true;
            }
            for rec in records.iter().filter(|r| r.is_fde) {
                let identity = (|| {
                    let iloc = rec.iloc_offset? as u64;
                    let index = relas.partition_point(|r| r.offset < iloc);
                    let loc = relas.get(index).filter(|r| r.offset == iloc)?;
                    let sym = obj.symbols.get(loc.sym_idx as usize)?;
                    let code = sym.shndx as usize;
                    if !sym.is_local() || !is_foldable(&obj.sections.get(code)?.name) {
                        return None;
                    }
                    // Multiple FDEs/interior entries are deliberately opaque;
                    // their association needs a richer range identity.
                    if sym.value.checked_add_signed(loc.addend) != Some(0) {
                        return None;
                    }
                    let id = data
                        .get(rec.id_offset..rec.id_offset.checked_add(rec.id_size)?)?
                        .iter()
                        .enumerate()
                        .fold(0u64, |v, (n, b)| v | (u64::from(*b) << (8 * n)));
                    let cie_at = rec.id_offset.checked_sub(usize::try_from(id).ok()?)?;
                    let cie =
                        records.get(records.binary_search_by_key(&cie_at, |r| r.start).ok()?)?;
                    if cie.is_fde {
                        return None;
                    }
                    Some((
                        code,
                        Identity::Exact {
                            cie: reader.blob(si, cie.start..cie.end, code, None, false)?,
                            fde: reader.blob(
                                si,
                                rec.start..rec.end,
                                code,
                                Some((rec.id_offset, rec.id_size)),
                                false,
                            )?,
                        },
                    ))
                })();
                match identity {
                    Some((code, key)) => match out.entry((oi, code)) {
                        Entry::Vacant(e) => {
                            e.insert(key);
                        }
                        Entry::Occupied(mut e) => {
                            e.insert(Identity::Opaque((oi, code)));
                        }
                    },
                    None => malformed = true,
                }
            }
        }
        if malformed {
            for (si, sec) in obj.sections.iter().enumerate() {
                if is_foldable(&sec.name) {
                    out.insert((oi, si), Identity::Opaque((oi, si)));
                }
            }
        }
    }
    out
}
