//! Version requirements (`.gnu.version` / `.gnu.version_r`) for imported
//! symbols, shared by every lccc-ld dynamic output (executables and shared
//! objects on all targets).
//!
//! A reference resolved against a versioned export must carry that version,
//! exactly as GNU ld records it: the import's `.gnu.version` entry holds an
//! index that a `Vernaux` under the provider's `Verneed` defines.  Without
//! the tables ld.so binds the reference to whatever version is the default
//! *at run time*, so a program linked against `foo@@V1` silently picks up an
//! incompatible `foo@@V2` after a library upgrade, and glibc cannot diagnose
//! a too-old library ("version `GLIBC_2.38' not found").
//!
//! `Elf32_Verneed`/`Elf32_Vernaux` and their 64-bit forms are field-for-field
//! identical (`Half`/`Word` only), so one encoder serves ELFCLASS32 and
//! ELFCLASS64; every lccc-ld target is little-endian.

use std::collections::BTreeSet;

use crate::common::fx_hash::FxHashMap;

/// `.gnu.version` value of a defined (exported) symbol.
pub const VER_NDX_GLOBAL: u16 = 1;

/// Size of one `Verneed` and of one `Vernaux` record.
const RECORD: usize = 16;

/// The version-index assignment and the `.gnu.version_r` contents for one
/// output.  Build it before layout (its strings go into `.dynstr` and its
/// emptiness decides whether `DT_VERNEED`/`DT_VERSYM` exist, so the
/// `.dynamic` size never depends on later values).
#[derive(Debug, Default)]
pub struct VerneedPlan {
    /// `(soname, versions)` in `DT_NEEDED` order; versions sorted.
    files: Vec<(String, Vec<String>)>,
    index: FxHashMap<(String, String), u16>,
}

impl VerneedPlan {
    /// `imports` yields `(provider soname, version)` for every imported
    /// dynamic symbol; `needed` lists the `DT_NEEDED` sonames in emission
    /// order; `first_index` is the first free version index (`2` unless the
    /// output also defines versions, whose `Verdef` indices come first).
    ///
    /// Only providers that are actually `DT_NEEDED` get a `Verneed`: an index
    /// without a defining `Vernaux` would make ld.so read past the version
    /// array it sizes from `.gnu.version_r`, so such an import is left
    /// unversioned ([`VER_NDX_GLOBAL`]) instead.
    pub fn new<'a>(
        imports: impl IntoIterator<Item = (&'a str, &'a str)>,
        needed: &[String],
        first_index: u16,
    ) -> Result<Self, String> {
        let mut per_file: FxHashMap<&str, BTreeSet<&str>> = FxHashMap::default();
        for (soname, version) in imports {
            if !version.is_empty() && needed.iter().any(|n| n == soname) {
                per_file.entry(soname).or_default().insert(version);
            }
        }
        let mut plan = VerneedPlan::default();
        let mut next = first_index;
        for soname in needed {
            let Some(versions) = per_file.remove(soname.as_str()) else {
                continue; // no versioned import, or a duplicate NEEDED entry
            };
            let mut list = Vec::with_capacity(versions.len());
            for version in versions {
                plan.index
                    .insert((soname.clone(), version.to_string()), next);
                // `.gnu.version` entries are 15-bit (bit 15 is "hidden"), so
                // exhausting the index space is a link error, as in GNU ld,
                // never a silently unversioned binding.
                next = next.checked_add(1).filter(|n| *n < 0x8000).ok_or_else(|| {
                    format!("{soname}: more symbol versions than the 15-bit versym index holds")
                })?;
                list.push(version.to_string());
            }
            plan.files.push((soname.clone(), list));
        }
        Ok(plan)
    }

    /// True when no import needs a version (emit no tables and no tags).
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// `DT_VERNEEDNUM`: the number of `Verneed` records.
    pub fn file_count(&self) -> u64 {
        self.files.len() as u64
    }

    /// Every string `.gnu.version_r` references: provider sonames (normally
    /// already in `.dynstr` for `DT_NEEDED`) and version names.
    pub fn strings(&self) -> impl Iterator<Item = &str> {
        self.files
            .iter()
            .flat_map(|(f, vs)| std::iter::once(f.as_str()).chain(vs.iter().map(String::as_str)))
    }

    /// The `.gnu.version` entry of an import provided by `soname` with
    /// `version` (`None`: unversioned export) — [`VER_NDX_GLOBAL`] when the
    /// pair has no `Vernaux`.
    pub fn index_of(&self, soname: &str, version: Option<&str>) -> u16 {
        version
            .and_then(|v| self.index.get(&(soname.to_string(), v.to_string())))
            .copied()
            .unwrap_or(VER_NDX_GLOBAL)
    }

    /// Encode `.gnu.version_r`; `str_off` maps a string from [`strings`]
    /// to its `.dynstr` offset.
    ///
    /// [`strings`]: VerneedPlan::strings
    pub fn encode(&self, mut str_off: impl FnMut(&str) -> u32) -> Vec<u8> {
        let mut out = Vec::new();
        for (fi, (soname, versions)) in self.files.iter().enumerate() {
            let last_file = fi + 1 == self.files.len();
            out.extend_from_slice(&1u16.to_le_bytes()); // vn_version
            out.extend_from_slice(&(versions.len() as u16).to_le_bytes()); // vn_cnt
            out.extend_from_slice(&str_off(soname).to_le_bytes()); // vn_file
            out.extend_from_slice(&(RECORD as u32).to_le_bytes()); // vn_aux
            let next = if last_file {
                0
            } else {
                RECORD * (1 + versions.len())
            };
            out.extend_from_slice(&(next as u32).to_le_bytes()); // vn_next
            for (vi, version) in versions.iter().enumerate() {
                out.extend_from_slice(&super::sysv_hash(version.as_bytes()).to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes()); // vna_flags
                out.extend_from_slice(
                    &self.index[&(soname.clone(), version.clone())].to_le_bytes(),
                );
                out.extend_from_slice(&str_off(version).to_le_bytes()); // vna_name
                let next = if vi + 1 == versions.len() { 0 } else { RECORD };
                out.extend_from_slice(&(next as u32).to_le_bytes()); // vna_next
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    #[test]
    fn layout_matches_the_gabi_chain() {
        let needed = vec!["libc.so.6".to_string(), "libm.so.6".to_string()];
        let imports = [
            ("libm.so.6", "GLIBC_2.29"),
            ("libc.so.6", "GLIBC_2.34"),
            ("libc.so.6", "GLIBC_2.2.5"),
            ("libc.so.6", "GLIBC_2.34"),
        ];
        let plan = VerneedPlan::new(imports, &needed, 2).unwrap();
        // NEEDED order, versions sorted, indices dense from 2.
        assert_eq!(plan.index_of("libc.so.6", Some("GLIBC_2.2.5")), 2);
        assert_eq!(plan.index_of("libc.so.6", Some("GLIBC_2.34")), 3);
        assert_eq!(plan.index_of("libm.so.6", Some("GLIBC_2.29")), 4);
        assert_eq!(plan.index_of("libc.so.6", None), VER_NDX_GLOBAL);
        assert_eq!(plan.file_count(), 2);
        let strs: Vec<&str> = plan.strings().collect();
        let b = plan.encode(|s| 100 + strs.iter().position(|x| *x == s).unwrap() as u32);
        assert_eq!(b.len(), 16 * (2 + 3));
        // libc Verneed: 2 aux, next skips header + 2 aux.
        assert_eq!((u16_at(&b, 0), u16_at(&b, 2)), (1, 2));
        assert_eq!((u32_at(&b, 8), u32_at(&b, 12)), (16, 48));
        assert_eq!(u32_at(&b, 16), super::super::sysv_hash(b"GLIBC_2.2.5"));
        assert_eq!((u16_at(&b, 22), u32_at(&b, 28)), (2, 16));
        assert_eq!((u16_at(&b, 38), u32_at(&b, 44)), (3, 0));
        // libm Verneed is last.
        assert_eq!((u16_at(&b, 50), u32_at(&b, 60)), (1, 0));
        assert_eq!(u16_at(&b, 70), 4);
    }

    #[test]
    fn providers_outside_needed_stay_unversioned() {
        let needed = vec!["libc.so.6".to_string()];
        let plan = VerneedPlan::new([("libgone.so", "V1"), ("libc.so.6", "")], &needed, 2).unwrap();
        assert!(plan.is_empty());
        assert_eq!(plan.index_of("libgone.so", Some("V1")), VER_NDX_GLOBAL);
        assert!(plan.encode(|_| 0).is_empty());
    }

    #[test]
    fn indices_follow_the_outputs_own_verdefs() {
        let needed = vec!["libc.so.6".to_string(), "libc.so.6".to_string()];
        let plan = VerneedPlan::new([("libc.so.6", "GLIBC_2.2.5")], &needed, 4).unwrap();
        assert_eq!(plan.index_of("libc.so.6", Some("GLIBC_2.2.5")), 4);
        assert_eq!(
            plan.file_count(),
            1,
            "duplicate NEEDED entries share one Verneed"
        );
    }
}
