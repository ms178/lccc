//! The RISC-V global offset table: one 8-byte slot per address or
//! initial-exec TP offset, two per general-dynamic `tls_index` (module,
//! offset). Slots are keyed by symbol (locals object-scoped, see
//! `got_sym_key`) and by access kind, because one object may reach the
//! same TLS variable both ways and each way needs its own slot contents.

use crate::common::fx_hash::FxHashMap;

use super::relocations::{R_RISCV_GOT_HI20, R_RISCV_TLS_GD_HI20, R_RISCV_TLS_GOT_HI20};

/// The `glibc`/psABI bias of a RISC-V `DTPREL` value: `__tls_get_addr`
/// adds it back, so a module-relative offset is stored as `offset - 0x800`.
pub const TLS_DTV_OFFSET: u64 = 0x800;

/// Dynamic relocation types the loader applies to TLS GOT slots.
pub const R_RISCV_TLS_DTPMOD64: u64 = 7;
pub const R_RISCV_TLS_DTPREL64: u64 = 9;
pub const R_RISCV_TLS_TPREL64: u64 = 11;

/// How a GOT slot is reached, which fixes what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GotKind {
    /// The symbol's address (`R_RISCV_GOT_HI20`).
    Addr,
    /// The symbol's offset from the thread pointer (`R_RISCV_TLS_GOT_HI20`).
    TlsIe,
    /// A `tls_index` -- module id, then `DTPREL` offset
    /// (`R_RISCV_TLS_GD_HI20`).
    TlsGd,
}

impl GotKind {
    /// The kind of slot a relocation type addresses, if any.
    pub fn of(rela_type: u32) -> Option<Self> {
        match rela_type {
            R_RISCV_GOT_HI20 => Some(Self::Addr),
            R_RISCV_TLS_GOT_HI20 => Some(Self::TlsIe),
            R_RISCV_TLS_GD_HI20 => Some(Self::TlsGd),
            _ => None,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn bytes(self) -> u64 {
        if self == Self::TlsGd { 16 } else { 8 }
    }
}

/// Slot offsets from the start of `.got`, in first-reference order.
#[derive(Debug, Default)]
pub struct GotLayout {
    entries: Vec<(String, GotKind, u64)>,
    by_key: FxHashMap<String, [Option<u64>; 3]>,
    size: u64,
}

impl GotLayout {
    /// Lays out one slot (or pair) per distinct `(key, kind)`.
    pub fn new(wanted: impl IntoIterator<Item = (String, GotKind)>) -> Self {
        let mut layout = Self::default();
        for (key, kind) in wanted {
            let slots = layout.by_key.entry(key.clone()).or_default();
            if slots[kind.index()].is_none() {
                slots[kind.index()] = Some(layout.size);
                layout.entries.push((key, kind, layout.size));
                layout.size += kind.bytes();
            }
        }
        layout
    }

    /// The offset of `key`'s slot of `kind`, if it has one.
    pub fn offset(&self, key: &str, kind: GotKind) -> Option<u64> {
        self.by_key.get(key).and_then(|s| s[kind.index()])
    }

    /// Every slot: key, kind, offset.
    pub fn entries(&self) -> impl Iterator<Item = (&str, GotKind, u64)> {
        self.entries
            .iter()
            .map(|(k, kind, off)| (k.as_str(), *kind, *off))
    }

    /// The size of `.got` in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gd_takes_a_slot_pair_and_kinds_do_not_share_slots() {
        let got = GotLayout::new([
            ("a".to_string(), GotKind::Addr),
            ("t".to_string(), GotKind::TlsGd),
            ("t".to_string(), GotKind::TlsIe),
            ("a".to_string(), GotKind::Addr),
            ("t".to_string(), GotKind::TlsGd),
        ]);
        assert_eq!(got.offset("a", GotKind::Addr), Some(0));
        assert_eq!(got.offset("t", GotKind::TlsGd), Some(8));
        assert_eq!(got.offset("t", GotKind::TlsIe), Some(24));
        assert_eq!(got.offset("t", GotKind::Addr), None);
        assert_eq!(got.size(), 32);
        assert_eq!(got.entries().count(), 3);
    }

    #[test]
    fn relocation_kinds() {
        assert_eq!(GotKind::of(R_RISCV_GOT_HI20), Some(GotKind::Addr));
        assert_eq!(GotKind::of(R_RISCV_TLS_GOT_HI20), Some(GotKind::TlsIe));
        assert_eq!(GotKind::of(R_RISCV_TLS_GD_HI20), Some(GotKind::TlsGd));
        assert_eq!(GotKind::of(1), None);
    }
}
