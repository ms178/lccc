//! The `.dynamic` table shared by the i686 executable and shared-object
//! emitters.
//!
//! Both emitters must reserve the table's space before the layout assigns the
//! addresses it records.  Describing the table once — which entries exist is
//! fixed when the description is built, only their values are filled in
//! after layout — makes the reserved size and the written table agree by
//! construction; [`DynamicDesc::serialize`] still checks it.

use super::types::*;

/// Contents of `.dynamic`.  `Some` = the entry (group) is present.
#[derive(Clone, Debug, Default)]
pub(super) struct DynamicDesc {
    /// `DT_NEEDED` `.dynstr` offsets, in order.
    pub needed: Vec<u32>,
    /// Option-derived string entries (`DT_SONAME`, `DT_RPATH`/`DT_RUNPATH`):
    /// (tag, `.dynstr` offset).
    pub strings: Vec<(i32, u32)>,
    pub init: Option<u32>,
    pub fini: Option<u32>,
    /// (address, size) pairs.
    pub preinit_array: Option<(u32, u32)>,
    pub init_array: Option<(u32, u32)>,
    pub fini_array: Option<(u32, u32)>,
    pub hash: Option<u32>,
    pub gnu_hash: Option<u32>,
    pub strtab: u32,
    pub symtab: u32,
    pub strsz: u32,
    /// `DT_DEBUG` (executables only).
    pub debug: bool,
    /// (`DT_PLTGOT`, `DT_JMPREL`, `DT_PLTRELSZ`).
    pub plt: Option<(u32, u32, u32)>,
    /// (`DT_REL`, `DT_RELSZ`).
    pub rel: Option<(u32, u32)>,
    /// `DT_RELCOUNT`: leading `R_386_RELATIVE` entries of `.rel.dyn`.
    pub relcount: Option<u32>,
    /// (`DT_VERSYM`, `DT_VERNEED`, `DT_VERNEEDNUM`).
    pub verneed: Option<(u32, u32, u32)>,
    /// Option-derived numeric entries (`DT_SYMBOLIC`, `DT_TEXTREL`,
    /// `DT_FLAGS`, `DT_FLAGS_1`).
    pub numeric: Vec<(i32, u32)>,
}

pub(super) const DT_RELCOUNT: i32 = 0x6fff_fffa;

impl DynamicDesc {
    /// The entries in emission order, `DT_NULL` last.
    pub fn entries(&self) -> Vec<(i32, u32)> {
        let mut v: Vec<(i32, u32)> = Vec::new();
        v.extend(self.needed.iter().map(|&o| (DT_NEEDED, o)));
        v.extend(self.strings.iter().copied());
        if let Some(a) = self.init {
            v.push((DT_INIT, a));
        }
        if let Some(a) = self.fini {
            v.push((DT_FINI, a));
        }
        if let Some((a, s)) = self.preinit_array {
            v.push((DT_PREINIT_ARRAY, a));
            v.push((DT_PREINIT_ARRAYSZ, s));
        }
        if let Some((a, s)) = self.init_array {
            v.push((DT_INIT_ARRAY, a));
            v.push((DT_INIT_ARRAYSZ, s));
        }
        if let Some((a, s)) = self.fini_array {
            v.push((DT_FINI_ARRAY, a));
            v.push((DT_FINI_ARRAYSZ, s));
        }
        if let Some(a) = self.hash {
            v.push((DT_HASH, a));
        }
        if let Some(a) = self.gnu_hash {
            v.push((DT_GNU_HASH_TAG, a));
        }
        v.push((DT_STRTAB, self.strtab));
        v.push((DT_SYMTAB, self.symtab));
        v.push((DT_STRSZ, self.strsz));
        v.push((DT_SYMENT, 16));
        if self.debug {
            v.push((DT_DEBUG, 0));
        }
        if let Some((pltgot, jmprel, sz)) = self.plt {
            v.push((DT_PLTGOT, pltgot));
            v.push((DT_PLTRELSZ, sz));
            v.push((DT_PLTREL, DT_REL as u32));
            v.push((DT_JMPREL, jmprel));
        }
        if let Some((a, s)) = self.rel {
            v.push((DT_REL, a));
            v.push((DT_RELSZ, s));
            v.push((DT_RELENT, 8));
        }
        if let Some(n) = self.relcount {
            v.push((DT_RELCOUNT, n));
        }
        v.extend(self.numeric.iter().copied());
        if let Some((versym, verneed, num)) = self.verneed {
            v.push((DT_VERSYM, versym));
            v.push((DT_VERNEED, verneed));
            v.push((DT_VERNEEDNUM, num));
        }
        v.push((DT_NULL, 0));
        v
    }

    /// Size of the table in bytes.
    pub fn byte_size(&self) -> u32 {
        self.entries().len() as u32 * 8
    }

    /// Encode the table, checking it still fits the `reserved` bytes the
    /// layout set aside (a mismatch is a linker bug: an entry appeared or
    /// vanished between sizing and writing).
    pub fn serialize(&self, reserved: u32) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        for (tag, val) in self.entries() {
            push_dyn(&mut data, tag, val);
        }
        if data.len() as u32 != reserved {
            return Err(format!(
                "internal error: .dynamic is {} bytes but {} were reserved",
                data.len(),
                reserved
            ));
        }
        Ok(data)
    }
}

/// Split option-derived entries into string and numeric groups, resolving
/// strings through `offset_of` (the final `.dynstr`).
pub(super) fn option_entries(
    tags: &[(i32, super::options::DynValue)],
    offset_of: impl Fn(&str) -> u32,
) -> (Vec<(i32, u32)>, Vec<(i32, u32)>) {
    use super::options::DynValue;
    let mut strings = Vec::new();
    let mut numeric = Vec::new();
    for (tag, v) in tags {
        match v {
            DynValue::Str(s) => strings.push((*tag, offset_of(s))),
            DynValue::Num(n) => numeric.push((*tag, *n)),
        }
    }
    (strings, numeric)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_matches_serialized_table_for_every_presence_combination() {
        for mask in 0u32..(1 << 9) {
            let bit = |b: u32| mask & (1 << b) != 0;
            let d = DynamicDesc {
                needed: vec![1; (mask % 3) as usize],
                strings: if bit(0) { vec![(14, 5)] } else { vec![] },
                init: bit(1).then_some(0x1000),
                fini: bit(2).then_some(0x1100),
                preinit_array: bit(3).then_some((0x2000, 4)),
                init_array: bit(4).then_some((0x2004, 8)),
                fini_array: bit(4).then_some((0x200c, 4)),
                hash: bit(5).then_some(0x100),
                gnu_hash: (!bit(5) || bit(6)).then_some(0x200),
                debug: bit(6),
                plt: bit(7).then_some((0x3000, 0x400, 16)),
                rel: bit(8).then_some((0x500, 24)),
                relcount: bit(8).then_some(2),
                verneed: bit(7).then_some((0x600, 0x640, 1)),
                numeric: if bit(2) { vec![(30, 8)] } else { vec![] },
                ..Default::default()
            };
            let bytes = d.serialize(d.byte_size()).unwrap();
            assert_eq!(bytes.len() as u32, d.byte_size());
            // DT_NULL terminates, exactly once.
            let tags: Vec<i32> = bytes
                .chunks(8)
                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            assert_eq!(tags.last(), Some(&DT_NULL));
            assert_eq!(tags.iter().filter(|&&t| t == DT_NULL).count(), 1);
        }
    }

    #[test]
    fn size_mismatch_is_reported() {
        let d = DynamicDesc::default();
        assert!(d.serialize(d.byte_size() + 8).is_err());
    }
}
