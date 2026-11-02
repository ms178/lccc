//! The TLS GOT slots of an AArch64 shared object, which cannot relax any
//! access: an initial-exec slot holds a TP offset (R_AARCH64_TLS_TPREL64),
//! a general-dynamic one a `tls_index` (DTPMOD64, DTPREL64), a descriptor
//! one a `tlsdesc` the loader fills in in full (R_AARCH64_TLSDESC). The
//! descriptor relocations go in `.rela.dyn` because this emitter declares the
//! object bind-now unconditionally (`DF_BIND_NOW` and `DF_1_NOW`, see
//! `emit_shared`), so every descriptor is resolved at load time; the
//! `DT_JMPREL` form exists for lazy binding only. A symbol
//! another module may preempt is relocated by name; any other against this
//! module (symbol 0), its offset in the TLS template as the addend, a
//! DTPREL word written statically (AArch64 has no DTV bias).

use crate::backend::elf::STV_DEFAULT;
use crate::common::fx_hash::FxHashMap;

use super::elf::{ElfObject, Symbol};
use super::reloc::{
    R_AARCH64_TLSDESC_ADD_LO12, R_AARCH64_TLSDESC_ADR_PAGE21, R_AARCH64_TLSDESC_LD64_LO12,
    R_AARCH64_TLSGD_ADD_LO12_NC, R_AARCH64_TLSGD_ADR_PAGE21, R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21,
    R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC, got_key,
};
use super::types::GlobalSymbol;

pub const R_AARCH64_TLS_DTPMOD64: u64 = 1028;
pub const R_AARCH64_TLS_DTPREL64: u64 = 1029;
pub const R_AARCH64_TLS_TPREL64: u64 = 1030;
pub const R_AARCH64_TLSDESC: u64 = 1031;

/// The access model a TLS GOT relocation belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsKind {
    Ie,
    Gd,
    Desc,
}

impl TlsKind {
    /// The model of a relocation that addresses a TLS GOT slot.
    pub fn of(rela_type: u32) -> Option<Self> {
        match rela_type {
            R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 | R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC => {
                Some(Self::Ie)
            }
            R_AARCH64_TLSGD_ADR_PAGE21 | R_AARCH64_TLSGD_ADD_LO12_NC => Some(Self::Gd),
            R_AARCH64_TLSDESC_ADR_PAGE21
            | R_AARCH64_TLSDESC_LD64_LO12
            | R_AARCH64_TLSDESC_ADD_LO12 => Some(Self::Desc),
            _ => None,
        }
    }

    fn bytes(self) -> u64 {
        if self == Self::Ie { 8 } else { 16 }
    }
}

/// One slot (IE) or slot pair (GD, descriptor).
#[derive(Debug)]
pub struct TlsSlot {
    pub key: String,
    pub kind: TlsKind,
    pub obj_idx: usize,
    pub sym_idx: usize,
    /// Byte offset from the first TLS slot.
    pub offset: u64,
    /// Relocated by name (default visibility, or undefined here).
    pub preemptible: bool,
}

/// Every TLS slot, in first-reference order.
#[derive(Debug, Default)]
pub struct TlsGot {
    pub slots: Vec<TlsSlot>,
    pub size: u64,
}

impl TlsGot {
    pub fn collect(objects: &[ElfObject], globals: &FxHashMap<String, GlobalSymbol>) -> Self {
        let mut got = Self::default();
        for (obj_idx, obj) in objects.iter().enumerate() {
            for relas in &obj.relocations {
                for rela in relas {
                    let (Some(kind), Some(sym)) = (
                        TlsKind::of(rela.rela_type),
                        obj.symbols.get(rela.sym_idx as usize),
                    ) else {
                        continue;
                    };
                    let key = got_key(obj_idx, sym);
                    if got.slot(&key, kind).is_some() {
                        continue;
                    }
                    got.slots.push(TlsSlot {
                        preemptible: preemptible(sym, globals),
                        key,
                        kind,
                        obj_idx,
                        sym_idx: rela.sym_idx as usize,
                        offset: got.size,
                    });
                    got.size += kind.bytes();
                }
            }
        }
        got
    }

    pub fn slot(&self, key: &str, kind: TlsKind) -> Option<&TlsSlot> {
        self.slots.iter().find(|s| s.kind == kind && s.key == key)
    }

    /// How many `.rela.dyn` entries the slots need.
    pub fn dynamic_reloc_count(&self) -> usize {
        self.slots
            .iter()
            .map(|s| usize::from(s.kind == TlsKind::Gd && s.preemptible) + 1)
            .sum()
    }
}

fn preemptible(sym: &Symbol, globals: &FxHashMap<String, GlobalSymbol>) -> bool {
    if sym.is_local() {
        return false;
    }
    globals
        .get(sym.name.as_str())
        .is_none_or(|g| g.visibility & 0x3 == STV_DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_and_slot_sizes() {
        assert_eq!(
            TlsKind::of(R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21),
            Some(TlsKind::Ie)
        );
        assert_eq!(TlsKind::of(R_AARCH64_TLSGD_ADD_LO12_NC), Some(TlsKind::Gd));
        assert_eq!(TlsKind::of(R_AARCH64_TLSDESC_ADD_LO12), Some(TlsKind::Desc));
        assert_eq!(TlsKind::of(569), None); // TLSDESC_CALL addresses no slot
        assert_eq!(TlsKind::Ie.bytes(), 8);
        assert_eq!(TlsKind::Gd.bytes(), 16);
        assert_eq!(TlsKind::Desc.bytes(), 16);
    }

    #[test]
    fn dynamic_reloc_count_per_model() {
        let slot = |kind, preemptible| TlsSlot {
            key: String::new(),
            kind,
            obj_idx: 0,
            sym_idx: 0,
            offset: 0,
            preemptible,
        };
        let got = TlsGot {
            slots: vec![
                slot(TlsKind::Gd, true),
                slot(TlsKind::Gd, false),
                slot(TlsKind::Desc, true),
                slot(TlsKind::Ie, false),
            ],
            size: 56,
        };
        assert_eq!(got.dynamic_reloc_count(), 2 + 1 + 1 + 1);
    }
}
